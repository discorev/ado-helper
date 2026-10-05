import Foundation

public protocol ADOHTTPTransport: Sendable {
    func data(for request: URLRequest) async throws -> (Data, HTTPURLResponse)
}

public enum ADOClientError: Error, LocalizedError, Equatable {
    case invalidRequest
    case invalidArgument(String)
    case transportFailure
    case unsafeRedirect
    case httpStatus(Int, method: String, path: String)
    case invalidResponse
    case paginationFailure

    public var errorDescription: String? {
        switch self {
        case .invalidRequest:
            return "Could not construct a valid Azure DevOps request."
        case .invalidArgument(let message):
            return message
        case .transportFailure:
            return "The Azure DevOps request could not be completed."
        case .unsafeRedirect:
            return "Azure DevOps redirected the request. Redirects are disabled for authenticated requests."
        case .httpStatus(let status, let method, let path):
            if status == 401 {
                return "Azure DevOps rejected authentication (HTTP 401). Renew the affected profile with 'ado auth update NAME'."
            }
            if status == 403 {
                return "Azure DevOps denied access (HTTP 403). Check the profile's PAT scopes and account permissions for this repository."
            }
            return "Azure DevOps returned HTTP \(status) for \(method) \(path)."
        case .invalidResponse:
            return "Azure DevOps returned a response in an unexpected format."
        case .paginationFailure:
            return "Azure DevOps returned an invalid or repeating pagination cursor."
        }
    }
}

public final class ADOClient {
    private static let apiVersion = "7.1"
    private let organization: Organization
    private let authorization: String
    private let transport: any ADOHTTPTransport

    public convenience init(organization: Organization, token: String) {
        self.init(organization: organization, token: token, transport: URLSessionADOHTTPTransport())
    }

    public init(organization: Organization, token: String, transport: any ADOHTTPTransport) {
        self.organization = organization
        authorization = "Basic " + Data(":\(token)".utf8).base64EncodedString()
        self.transport = transport
    }

    public func identity() async throws -> ADOIdentity {
        let object = try await getObject(
            path: ["_apis", "connectionData"],
            query: [
                URLQueryItem(name: "connectOptions", value: "1"),
                URLQueryItem(name: "lastChangeId", value: "-1"),
                URLQueryItem(name: "lastChangeId64", value: "-1")
            ],
            includesAPIVersion: false
        )
        guard let user = object["authenticatedUser"] as? [String: Any],
              let id = nonemptyString(user["id"]),
              let displayName = nonemptyString(user["providerDisplayName"])
                ?? nonemptyString(user["displayName"]),
              let uniqueName = Self.identityUniqueName(from: user) else {
            throw ADOClientError.invalidResponse
        }
        return ADOIdentity(id: id, displayName: displayName, uniqueName: uniqueName)
    }

    public func pullRequest(id: Int) async throws -> [String: Any] {
        guard id > 0 else {
            throw ADOClientError.invalidArgument("Pull request ID must be greater than zero.")
        }
        return try await getObject(path: ["_apis", "git", "pullrequests", String(id)])
    }

    public func fileContent(
        project: String,
        repository: String,
        path: String,
        commit: String
    ) async throws -> String {
        try validateResource(project: project, repository: repository, id: 1)
        guard Self.isCommitSHA(commit) else {
            throw ADOClientError.invalidArgument("Commit must be a 40-character hexadecimal SHA.")
        }

        let normalizedPath = path.hasPrefix("/") ? path : "/" + path
        let components = normalizedPath.split(separator: "/", omittingEmptySubsequences: false)
        guard normalizedPath.count > 1,
              !normalizedPath.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              !normalizedPath.contains("//"),
              !components.contains(where: { $0 == "." || $0 == ".." }) else {
            throw ADOClientError.invalidArgument("File path must be a valid repository-relative path.")
        }

        let object = try await getObject(
            path: [project, "_apis", "git", "repositories", repository, "items"],
            query: [
                URLQueryItem(name: "path", value: normalizedPath),
                URLQueryItem(name: "includeContent", value: "true"),
                URLQueryItem(name: "includeContentMetadata", value: "true"),
                URLQueryItem(name: "versionDescriptor.versionType", value: "commit"),
                URLQueryItem(name: "versionDescriptor.version", value: commit.lowercased())
            ]
        )
        let metadata = object["contentMetadata"] as? [String: Any]
        guard object["isFolder"] as? Bool != true,
              (object["gitObjectType"] as? String)?.lowercased() != "tree",
              metadata?["isBinary"] as? Bool != true,
              object["isBinary"] as? Bool != true,
              let content = object["content"] as? String else {
            throw ADOClientError.invalidResponse
        }
        return content
    }

    public func threads(project: String, repository: String, id: Int) async throws -> [[String: Any]] {
        try validateResource(project: project, repository: repository, id: id)
        let path = repositoryPath(project: project, repository: repository, id: id) + ["threads"]
        return try await continuationPagedValues(path: path)
    }

    public func iterations(project: String, repository: String, id: Int) async throws -> [[String: Any]] {
        try validateResource(project: project, repository: repository, id: id)
        let path = repositoryPath(project: project, repository: repository, id: id) + ["iterations"]
        return try await continuationPagedValues(path: path)
    }

    public func changes(
        project: String,
        repository: String,
        id: Int,
        iteration: Int
    ) async throws -> [[String: Any]] {
        try validateResource(project: project, repository: repository, id: id)
        guard iteration > 0 else {
            throw ADOClientError.invalidArgument("Iteration must be greater than zero.")
        }

        let path = repositoryPath(project: project, repository: repository, id: id)
            + ["iterations", String(iteration), "changes"]
        var results: [[String: Any]] = []
        var skip: Int?
        var top = 2_000
        var seenSkips: Set<Int> = []

        while true {
            var query = [URLQueryItem(name: "$top", value: String(top))]
            if let skip {
                guard skip > 0, seenSkips.insert(skip).inserted else {
                    throw ADOClientError.paginationFailure
                }
                query.append(URLQueryItem(name: "$skip", value: String(skip)))
            }

            let object = try await getObject(path: path, query: query)
            guard let page = object["changeEntries"] as? [[String: Any]] else {
                throw ADOClientError.invalidResponse
            }
            results.append(contentsOf: page)

            guard let nextSkip = integer(object["nextSkip"]), nextSkip != 0 else {
                break
            }
            let nextTop = integer(object["nextTop"]) ?? 2_000
            guard nextSkip > 0, nextTop > 0, nextTop <= 2_000 else {
                throw ADOClientError.paginationFailure
            }
            skip = nextSkip
            top = nextTop
        }
        return results
    }

    public func listPullRequests(
        project: String,
        repository: String,
        sourceBranch: String?
    ) async throws -> [[String: Any]] {
        try validateResource(project: project, repository: repository, id: 1)
        if let sourceBranch, sourceBranch.isEmpty {
            throw ADOClientError.invalidArgument("Source branch must not be empty.")
        }

        let pageSize = 100
        let path = [project, "_apis", "git", "repositories", repository, "pullrequests"]
        var results: [[String: Any]] = []
        var seenFullPages: Set<Data> = []
        var skip = 0

        while true {
            var query = [
                URLQueryItem(name: "$top", value: String(pageSize)),
                URLQueryItem(name: "$skip", value: String(skip)),
                URLQueryItem(name: "searchCriteria.status", value: "active")
            ]
            if let sourceBranch {
                query.append(URLQueryItem(name: "searchCriteria.sourceRefName", value: sourceBranch))
            }
            let object = try await getObject(path: path, query: query)
            guard let page = object["value"] as? [[String: Any]] else {
                throw ADOClientError.invalidResponse
            }
            results.append(contentsOf: page)
            guard page.count == pageSize else { break }
            guard let fingerprint = try? JSONSerialization.data(
                withJSONObject: page,
                options: [.sortedKeys]
            ), seenFullPages.insert(fingerprint).inserted else {
                throw ADOClientError.paginationFailure
            }
            let (next, overflow) = skip.addingReportingOverflow(pageSize)
            guard !overflow else { throw ADOClientError.paginationFailure }
            skip = next
        }
        return results
    }

    public func createThread(
        project: String,
        repository: String,
        id: Int,
        body: String,
        path: String,
        startLine: Int,
        endLine: Int,
        side: String,
        iteration: Int,
        changeTrackingID: Int
    ) async throws -> [String: Any] {
        try validateResource(project: project, repository: repository, id: id)
        guard !body.isEmpty else {
            throw ADOClientError.invalidArgument("Comment body must not be empty.")
        }
        guard !path.isEmpty else {
            throw ADOClientError.invalidArgument("Comment path must not be empty.")
        }
        guard startLine > 0, endLine >= startLine else {
            throw ADOClientError.invalidArgument("Comment line range must use positive lines with the end at or after the start.")
        }
        guard side == "left" || side == "right" else {
            throw ADOClientError.invalidArgument("Comment side must be left or right.")
        }
        guard iteration > 0, changeTrackingID > 0 else {
            throw ADOClientError.invalidArgument("Iteration and change tracking ID must be greater than zero.")
        }

        let start: [String: Any] = ["line": startLine, "offset": 1]
        let end: [String: Any] = ["line": endLine, "offset": 1]
        var threadContext: [String: Any] = [
            "filePath": path,
            "leftFileStart": NSNull(),
            "leftFileEnd": NSNull(),
            "rightFileStart": NSNull(),
            "rightFileEnd": NSNull()
        ]
        threadContext["\(side)FileStart"] = start
        threadContext["\(side)FileEnd"] = end

        let payload: [String: Any] = [
            "comments": [[
                "parentCommentId": 0,
                "content": body,
                "commentType": 1
            ]],
            "status": 1,
            "threadContext": threadContext,
            "pullRequestThreadContext": [
                "changeTrackingId": changeTrackingID,
                "iterationContext": [
                    "firstComparingIteration": iteration,
                    "secondComparingIteration": iteration
                ]
            ]
        ]
        let requestPath = repositoryPath(project: project, repository: repository, id: id) + ["threads"]
        return try await sendObject(method: "POST", path: requestPath, body: payload)
    }

    private func repositoryPath(project: String, repository: String, id: Int) -> [String] {
        [project, "_apis", "git", "repositories", repository, "pullRequests", String(id)]
    }

    private func validateResource(project: String, repository: String, id: Int) throws {
        guard !project.isEmpty, !repository.isEmpty else {
            throw ADOClientError.invalidArgument("Project and repository must not be empty.")
        }
        guard id > 0 else {
            throw ADOClientError.invalidArgument("Pull request ID must be greater than zero.")
        }
    }

    private func continuationPagedValues(path: [String]) async throws -> [[String: Any]] {
        var results: [[String: Any]] = []
        var continuationToken: String?
        var seenTokens: Set<String> = []

        while true {
            var query: [URLQueryItem] = []
            if let continuationToken {
                guard !continuationToken.isEmpty, seenTokens.insert(continuationToken).inserted else {
                    throw ADOClientError.paginationFailure
                }
                query.append(URLQueryItem(name: "continuationToken", value: continuationToken))
            }
            let (object, response) = try await getObjectAndResponse(path: path, query: query)
            guard let page = object["value"] as? [[String: Any]] else {
                throw ADOClientError.invalidResponse
            }
            results.append(contentsOf: page)
            continuationToken = response.value(forHTTPHeaderField: "x-ms-continuationtoken")
            if continuationToken == nil { break }
        }
        return results
    }

    private func getObject(
        path: [String],
        query: [URLQueryItem] = [],
        includesAPIVersion: Bool = true
    ) async throws -> [String: Any] {
        let (object, _) = try await getObjectAndResponse(
            path: path,
            query: query,
            includesAPIVersion: includesAPIVersion
        )
        return object
    }

    private func getObjectAndResponse(
        path: [String],
        query: [URLQueryItem] = [],
        includesAPIVersion: Bool = true
    ) async throws -> ([String: Any], HTTPURLResponse) {
        let request = try makeRequest(
            method: "GET",
            path: path,
            query: query,
            includesAPIVersion: includesAPIVersion
        )
        let (data, response) = try await perform(request)
        return (try decodeObject(data), response)
    }

    private func sendObject(
        method: String,
        path: [String],
        body: [String: Any]
    ) async throws -> [String: Any] {
        guard JSONSerialization.isValidJSONObject(body) else {
            throw ADOClientError.invalidRequest
        }
        var request = try makeRequest(method: method, path: path)
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let (data, _) = try await perform(request)
        return try decodeObject(data)
    }

    private func makeRequest(
        method: String,
        path: [String],
        query: [URLQueryItem] = [],
        includesAPIVersion: Bool = true
    ) throws -> URLRequest {
        var components = URLComponents()
        components.scheme = "https"
        components.host = "dev.azure.com"
        components.percentEncodedPath = "/" + ([organization.name] + path)
            .map(Self.percentEncodePathSegment)
            .joined(separator: "/")
        var queryItems = query
        if includesAPIVersion {
            queryItems.append(URLQueryItem(name: "api-version", value: Self.apiVersion))
        }
        components.queryItems = queryItems.isEmpty ? nil : queryItems
        guard let url = components.url else { throw ADOClientError.invalidRequest }

        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue(authorization, forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return request
    }

    private func perform(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let data: Data
        let response: HTTPURLResponse
        do {
            (data, response) = try await transport.data(for: request)
        } catch let error as ADOClientError {
            throw error
        } catch {
            throw ADOClientError.transportFailure
        }

        guard response.url?.absoluteString == request.url?.absoluteString else {
            throw ADOClientError.unsafeRedirect
        }
        guard (200..<300).contains(response.statusCode) else {
            throw ADOClientError.httpStatus(
                response.statusCode,
                method: request.httpMethod ?? "REQUEST",
                path: request.url?.path ?? "/"
            )
        }
        return (data, response)
    }

    private func decodeObject(_ data: Data) throws -> [String: Any] {
        do {
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw ADOClientError.invalidResponse
            }
            return object
        } catch let error as ADOClientError {
            throw error
        } catch {
            throw ADOClientError.invalidResponse
        }
    }

    private static func percentEncodePathSegment(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: pathSegmentAllowed) ?? ""
    }

    private static let pathSegmentAllowed = CharacterSet(
        charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
    )

    private static func isCommitSHA(_ value: String) -> Bool {
        value.utf8.count == 40 && value.utf8.allSatisfy {
            (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
        }
    }

    private static func identityUniqueName(from user: [String: Any]) -> String? {
        if let uniqueName = nonemptyString(user["uniqueName"]) {
            return uniqueName
        }
        guard let properties = user["properties"] as? [String: Any],
              let account = properties["Account"] as? [String: Any] else {
            return nil
        }
        return nonemptyString(account["$value"]) ?? nonemptyString(account["value"])
    }
}

private func nonemptyString(_ value: Any?) -> String? {
    guard let string = value as? String, !string.isEmpty else { return nil }
    return string
}

private func integer(_ value: Any?) -> Int? {
    if let number = value as? NSNumber { return number.intValue }
    return value as? Int
}

private final class ADOURLSessionRedirectDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        // Authenticated requests never follow redirects. This avoids replaying a POST or
        // reusing a PAT on a different organization path even when the host is unchanged.
        completionHandler(nil)
    }
}

private final class URLSessionADOHTTPTransport: ADOHTTPTransport, @unchecked Sendable {
    private let session: URLSession

    init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.urlCredentialStorage = nil
        session = URLSession(
            configuration: configuration,
            delegate: ADOURLSessionRedirectDelegate(),
            delegateQueue: nil
        )
    }

    func data(for request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let (data, response) = try await session.data(for: request)
        guard let response = response as? HTTPURLResponse else {
            throw ADOClientError.invalidResponse
        }
        return (data, response)
    }
}
