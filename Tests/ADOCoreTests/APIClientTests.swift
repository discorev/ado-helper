import Foundation
import XCTest
@testable import ADOCore

final class APIClientTests: XCTestCase {
    func testIdentityUsesAccountPropertyFallback() async throws {
        let transport = MockTransport { request, _ in
            XCTAssertEqual(request.url?.path, "/acme/_apis/connectionData")
            XCTAssertEqual(query(request.url)["connectOptions"], "1")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Basic OnRlc3QtdG9rZW4=")
            return response(
                request,
                json: [
                    "authenticatedUser": [
                        "id": "identity-id",
                        "providerDisplayName": "Ada Lovelace",
                        "properties": ["Account": ["$value": "ada@example.test"]]
                    ]
                ]
            )
        }
        let client = ADOClient(organization: try Organization("acme"), token: "test-token", transport: transport)

        let identity = try await client.identity()

        XCTAssertEqual(identity, ADOIdentity(id: "identity-id", displayName: "Ada Lovelace", uniqueName: "ada@example.test"))
        XCTAssertEqual(transport.requestCount, 1)
    }

    func testPullRequestUsesOrganizationLevelEndpoint() async throws {
        let transport = MockTransport { request, _ in
            XCTAssertEqual(request.url?.path, "/acme/_apis/git/pullrequests/99")
            XCTAssertEqual(query(request.url)["api-version"], "7.1")
            return response(request, json: ["pullRequestId": 99])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        let pullRequest = try await client.pullRequest(id: 99)

        XCTAssertEqual(pullRequest["pullRequestId"] as? Int, 99)
    }

    func testFileContentPinsCommitAndEscapesPath() async throws {
        let commit = "ABCDEF0123456789ABCDEF0123456789ABCDEF01"
        let transport = MockTransport { request, _ in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/json")
            XCTAssertTrue(request.url?.absoluteString.contains(
                "/My%20Project/_apis/git/repositories/repo%2Fname/items"
            ) == true)
            let items = query(request.url)
            XCTAssertEqual(items["path"], "/Sources/A & B.swift")
            XCTAssertEqual(items["includeContent"], "true")
            XCTAssertEqual(items["includeContentMetadata"], "true")
            XCTAssertEqual(items["versionDescriptor.versionType"], "commit")
            XCTAssertEqual(items["versionDescriptor.version"], commit.lowercased())
            XCTAssertEqual(items["api-version"], "7.1")
            return response(request, json: [
                "gitObjectType": "blob",
                "isFolder": false,
                "contentMetadata": ["isBinary": false],
                "content": "first\nsecond\n"
            ])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        let content = try await client.fileContent(
            project: "My Project",
            repository: "repo/name",
            path: "Sources/A & B.swift",
            commit: commit
        )

        XCTAssertEqual(content, "first\nsecond\n")
        XCTAssertEqual(transport.requestCount, 1)
    }

    func testFileContentRejectsBinaryAndFolderResponses() async throws {
        let commit = String(repeating: "a", count: 40)
        let payloads: [[String: Any]] = [
            [
                "gitObjectType": "blob",
                "isFolder": false,
                "contentMetadata": ["isBinary": true],
                "content": "AAEC"
            ],
            [
                "gitObjectType": "tree",
                "isFolder": true,
                "contentMetadata": ["isBinary": false]
            ]
        ]

        for payload in payloads {
            let transport = MockTransport { request, _ in response(request, json: payload) }
            let client = ADOClient(
                organization: try Organization("acme"),
                token: "token",
                transport: transport
            )
            do {
                _ = try await client.fileContent(
                    project: "Project", repository: "Repo", path: "/file.bin", commit: commit
                )
                XCTFail("Expected non-text item rejection")
            } catch {
                XCTAssertEqual(error as? ADOClientError, .invalidResponse)
            }
        }
    }

    func testFileContentRejectsInvalidCommitWithoutRequest() async throws {
        let transport = MockTransport { request, _ in
            XCTFail("Invalid input must not issue a request")
            return response(request, json: [:])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        do {
            _ = try await client.fileContent(
                project: "Project", repository: "Repo", path: "/file.swift", commit: "main"
            )
            XCTFail("Expected commit validation failure")
        } catch {
            guard case .invalidArgument = error as? ADOClientError else {
                return XCTFail("Unexpected error: \(error)")
            }
        }
        XCTAssertEqual(transport.requestCount, 0)
    }

    func testPathComponentsAndSourceBranchAreEscapedAsData() async throws {
        let transport = MockTransport { request, _ in
            XCTAssertTrue(request.url?.absoluteString.contains("/A%20Project/_apis/git/repositories/repo%2Fname/pullrequests") == true)
            XCTAssertEqual(query(request.url)["searchCriteria.sourceRefName"], "refs/heads/feature & fix")
            XCTAssertEqual(query(request.url)["searchCriteria.status"], "active")
            return response(request, json: ["value": []])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        _ = try await client.listPullRequests(
            project: "A Project",
            repository: "repo/name",
            sourceBranch: "refs/heads/feature & fix"
        )
    }

    func testChangesFlattensNextSkipPages() async throws {
        let transport = MockTransport { request, index in
            let items = query(request.url)
            if index == 0 {
                XCTAssertEqual(items["$top"], "2000")
                XCTAssertNil(items["$skip"])
                return response(request, json: [
                    "changeEntries": [["changeTrackingId": 1]],
                    "nextSkip": 1,
                    "nextTop": 25
                ])
            }
            XCTAssertEqual(items["$top"], "25")
            XCTAssertEqual(items["$skip"], "1")
            return response(request, json: [
                "changeEntries": [["changeTrackingId": 2]],
                "nextSkip": 0,
                "nextTop": 0
            ])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        let changes = try await client.changes(project: "Project", repository: "Repo", id: 4, iteration: 3)

        XCTAssertEqual(changes.compactMap { $0["changeTrackingId"] as? Int }, [1, 2])
        XCTAssertEqual(transport.requestCount, 2)
    }

    func testThreadsFollowsContinuationHeaderAndEscapesToken() async throws {
        let token = "next token&opaque=yes"
        let transport = MockTransport { request, index in
            if index == 0 {
                XCTAssertNil(query(request.url)["continuationToken"])
                return response(
                    request,
                    json: ["value": [["id": 1]]],
                    headers: ["x-ms-continuationtoken": token]
                )
            }
            XCTAssertEqual(query(request.url)["continuationToken"], token)
            return response(request, json: ["value": [["id": 2]]])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        let threads = try await client.threads(project: "Project", repository: "Repo", id: 4)

        XCTAssertEqual(threads.compactMap { $0["id"] as? Int }, [1, 2])
        XCTAssertEqual(transport.requestCount, 2)
    }

    func testCreateThreadBuildsSideAndIterationContext() async throws {
        let transport = MockTransport { request, _ in
            XCTAssertEqual(request.httpMethod, "POST")
            let body = try XCTUnwrap(
                JSONSerialization.jsonObject(with: try XCTUnwrap(request.httpBody)) as? [String: Any]
            )
            let comments = try XCTUnwrap(body["comments"] as? [[String: Any]])
            XCTAssertEqual(comments.first?["content"] as? String, "Please adjust this")

            let context = try XCTUnwrap(body["threadContext"] as? [String: Any])
            XCTAssertEqual(context["filePath"] as? String, "/Sources/App.swift")
            XCTAssertTrue(context["leftFileStart"] is NSNull)
            XCTAssertEqual((context["rightFileStart"] as? [String: Any])?["line"] as? Int, 12)
            XCTAssertEqual((context["rightFileEnd"] as? [String: Any])?["line"] as? Int, 14)

            let pullRequestContext = try XCTUnwrap(body["pullRequestThreadContext"] as? [String: Any])
            XCTAssertEqual(pullRequestContext["changeTrackingId"] as? Int, 31)
            let iterations = try XCTUnwrap(pullRequestContext["iterationContext"] as? [String: Any])
            XCTAssertEqual(iterations["firstComparingIteration"] as? Int, 5)
            XCTAssertEqual(iterations["secondComparingIteration"] as? Int, 5)
            return response(request, json: ["id": 123])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "token", transport: transport)

        let thread = try await client.createThread(
            project: "Project",
            repository: "Repo",
            id: 4,
            body: "Please adjust this",
            path: "/Sources/App.swift",
            startLine: 12,
            endLine: 14,
            side: "right",
            iteration: 5,
            changeTrackingID: 31
        )

        XCTAssertEqual(thread["id"] as? Int, 123)
        XCTAssertEqual(transport.requestCount, 1)
    }

    func testMutationIsNotRetriedAndErrorDoesNotExposeBodyOrToken() async throws {
        let secret = "super-secret-token"
        let responseBody = "server said private-project-name and \(secret)"
        let transport = MockTransport { request, _ in
            response(request, status: 503, data: Data(responseBody.utf8))
        }
        let client = ADOClient(organization: try Organization("acme"), token: secret, transport: transport)

        do {
            _ = try await client.createThread(
                project: "Project", repository: "Repo", id: 4, body: "body",
                path: "/file", startLine: 1, endLine: 1, side: "left",
                iteration: 1, changeTrackingID: 1
            )
            XCTFail("Expected request to fail")
        } catch {
            let description = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
            XCTAssertTrue(description.contains("503"))
            XCTAssertFalse(description.contains(secret))
            XCTAssertFalse(description.contains("private-project-name"))
        }
        XCTAssertEqual(transport.requestCount, 1)
    }

    func testCrossHostResponseIsRejectedWithoutLeakingToken() async throws {
        let secret = "redirect-secret"
        let transport = MockTransport { request, _ in
            let evilURL = try XCTUnwrap(URL(string: "https://evil.example/capture"))
            let redirectResponse = try XCTUnwrap(HTTPURLResponse(
                url: evilURL,
                statusCode: 200,
                httpVersion: "HTTP/1.1",
                headerFields: nil
            ))
            return (Data("{\"value\":[]}".utf8), redirectResponse)
        }
        let client = ADOClient(organization: try Organization("acme"), token: secret, transport: transport)

        do {
            _ = try await client.threads(project: "Project", repository: "Repo", id: 4)
            XCTFail("Expected unsafe redirect rejection")
        } catch {
            XCTAssertEqual(error as? ADOClientError, .unsafeRedirect)
            XCTAssertFalse(error.localizedDescription.contains(secret))
        }
    }

    func testSameHostRedirectIsAlsoRejected() async throws {
        let transport = MockTransport { request, _ in
            var components = try XCTUnwrap(URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false))
            components.path = "/another-organization/_apis/git/pullrequests/4"
            let redirectedURL = try XCTUnwrap(components.url)
            let redirectedResponse = try XCTUnwrap(HTTPURLResponse(
                url: redirectedURL,
                statusCode: 200,
                httpVersion: "HTTP/1.1",
                headerFields: nil
            ))
            return (Data("{\"pullRequestId\":4}".utf8), redirectedResponse)
        }
        let client = ADOClient(organization: try Organization("acme"), token: "secret", transport: transport)

        do {
            _ = try await client.pullRequest(id: 4)
            XCTFail("Expected redirect rejection")
        } catch {
            XCTAssertEqual(error as? ADOClientError, .unsafeRedirect)
        }
    }

    func testListPullRequestsStopsWhenServerRepeatsAFullPage() async throws {
        let fullPage = (0..<100).map { ["pullRequestId": $0] }
        let transport = MockTransport { request, index in
            XCTAssertEqual(query(request.url)["$skip"], String(index * 100))
            return response(request, json: ["value": fullPage])
        }
        let client = ADOClient(organization: try Organization("acme"), token: "secret", transport: transport)

        do {
            _ = try await client.listPullRequests(project: "Project", repository: "Repo", sourceBranch: nil)
            XCTFail("Expected repeating page rejection")
        } catch {
            XCTAssertEqual(error as? ADOClientError, .paginationFailure)
        }
        XCTAssertEqual(transport.requestCount, 2)
    }
}

private final class MockTransport: ADOHTTPTransport, @unchecked Sendable {
    typealias Handler = (URLRequest, Int) throws -> (Data, HTTPURLResponse)

    private let lock = NSLock()
    private var count = 0
    private let handler: Handler

    init(handler: @escaping Handler) {
        self.handler = handler
    }

    var requestCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return count
    }

    func data(for request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        try handler(request, nextIndex())
    }

    private func nextIndex() -> Int {
        lock.lock()
        defer { lock.unlock() }
        let index = count
        count += 1
        return index
    }
}

private func response(
    _ request: URLRequest,
    status: Int = 200,
    json: [String: Any],
    headers: [String: String] = [:]
) -> (Data, HTTPURLResponse) {
    response(
        request,
        status: status,
        data: try! JSONSerialization.data(withJSONObject: json),
        headers: headers
    )
}

private func response(
    _ request: URLRequest,
    status: Int = 200,
    data: Data,
    headers: [String: String] = [:]
) -> (Data, HTTPURLResponse) {
    let httpResponse = HTTPURLResponse(
        url: request.url!,
        statusCode: status,
        httpVersion: "HTTP/1.1",
        headerFields: headers
    )!
    return (data, httpResponse)
}

private func query(_ url: URL?) -> [String: String] {
    guard let url, let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
        return [:]
    }
    return Dictionary(uniqueKeysWithValues: components.queryItems?.compactMap { item in
        item.value.map { (item.name, $0) }
    } ?? [])
}
