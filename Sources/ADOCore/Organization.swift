import Foundation

public enum OrganizationError: Error, LocalizedError, Equatable {
    case invalidOrganization

    public var errorDescription: String? {
        "Organization must be a bare Azure DevOps organization name or an HTTPS dev.azure.com/ORG or ORG.visualstudio.com URL."
    }
}

public struct Organization: Codable, Equatable, Hashable {
    public let name: String

    public var url: URL {
        URL(string: "https://dev.azure.com/\(name)")!
    }

    public init(_ value: String) throws {
        guard !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
            throw OrganizationError.invalidOrganization
        }

        if !value.contains("://") {
            guard Self.isValidName(value) else {
                throw OrganizationError.invalidOrganization
            }
            name = value.lowercased()
            return
        }

        guard let components = URLComponents(string: value),
              components.scheme?.lowercased() == "https",
              let host = components.host?.lowercased(),
              components.user == nil,
              components.password == nil,
              components.port == nil,
              components.query == nil,
              components.fragment == nil else {
            throw OrganizationError.invalidOrganization
        }

        let rawSegments = components.percentEncodedPath.split(separator: "/", omittingEmptySubsequences: true)
        let hasOnlyOptionalTrailingSlash = components.percentEncodedPath == "" ||
            components.percentEncodedPath == "/" ||
            !components.percentEncodedPath.dropFirst().contains("//")

        let candidate: String
        if host == "dev.azure.com" {
            guard hasOnlyOptionalTrailingSlash,
                  rawSegments.count == 1,
                  let decoded = String(rawSegments[0]).removingPercentEncoding else {
                throw OrganizationError.invalidOrganization
            }
            candidate = decoded
        } else if host.hasSuffix(".visualstudio.com") {
            guard rawSegments.isEmpty,
                  hasOnlyOptionalTrailingSlash else {
                throw OrganizationError.invalidOrganization
            }
            candidate = String(host.dropLast(".visualstudio.com".count))
            guard !candidate.contains(".") else {
                throw OrganizationError.invalidOrganization
            }
        } else {
            throw OrganizationError.invalidOrganization
        }

        guard Self.isValidName(candidate) else {
            throw OrganizationError.invalidOrganization
        }
        name = candidate.lowercased()
    }

    private enum CodingKeys: String, CodingKey { case name }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        do {
            self = try Organization(container.decode(String.self, forKey: .name))
        } catch {
            throw DecodingError.dataCorruptedError(
                forKey: .name,
                in: container,
                debugDescription: "Invalid Azure DevOps organization."
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(name, forKey: .name)
    }

    private static func isValidName(_ value: String) -> Bool {
        guard (1...50).contains(value.utf8.count),
              let first = value.utf8.first,
              let last = value.utf8.last,
              isASCIIAlphaNumeric(first),
              isASCIIAlphaNumeric(last) else {
            return false
        }
        return value.utf8.allSatisfy { isASCIIAlphaNumeric($0) || $0 == 45 }
    }

    private static func isASCIIAlphaNumeric(_ byte: UInt8) -> Bool {
        (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte)
    }
}

public enum PRLocatorError: Error, LocalizedError, Equatable {
    case invalidPullRequestURL

    public var errorDescription: String? {
        "Pull request target must be an HTTPS Azure DevOps pull request URL."
    }
}

public struct PRLocator: Equatable {
    public let organization: Organization
    public let project: String?
    public let repository: String?
    public let id: Int

    public init(organization: Organization, project: String?, repository: String?, id: Int) {
        self.organization = organization
        self.project = project
        self.repository = repository
        self.id = id
    }

    public init(url value: String) throws {
        guard value == value.trimmingCharacters(in: .whitespacesAndNewlines),
              let components = URLComponents(string: value),
              components.scheme?.lowercased() == "https",
              let host = components.host?.lowercased(),
              components.user == nil,
              components.password == nil,
              components.port == nil,
              !components.percentEncodedPath.dropFirst().contains("//") else {
            throw PRLocatorError.invalidPullRequestURL
        }

        let encodedSegments = components.percentEncodedPath.split(separator: "/", omittingEmptySubsequences: true)
        guard let segments = Self.decodePathSegments(encodedSegments) else {
            throw PRLocatorError.invalidPullRequestURL
        }

        let org: Organization
        let remaining: ArraySlice<String>
        if host == "dev.azure.com" {
            guard segments.count == 6 else { throw PRLocatorError.invalidPullRequestURL }
            org = try Organization(segments[0])
            remaining = segments[1...]
        } else if host.hasSuffix(".visualstudio.com") {
            guard segments.count == 5 || segments.count == 6 else {
                throw PRLocatorError.invalidPullRequestURL
            }
            let orgName = String(host.dropLast(".visualstudio.com".count))
            guard !orgName.contains(".") else { throw PRLocatorError.invalidPullRequestURL }
            org = try Organization(orgName)
            if segments.count == 6 {
                guard segments[0].caseInsensitiveCompare("DefaultCollection") == .orderedSame else {
                    throw PRLocatorError.invalidPullRequestURL
                }
                remaining = segments[1...]
            } else {
                remaining = segments[...]
            }
        } else {
            throw PRLocatorError.invalidPullRequestURL
        }

        let parts = Array(remaining)
        guard parts.count == 5,
              parts[1].lowercased() == "_git",
              parts[3].lowercased() == "pullrequest",
              let pullRequestID = Int(parts[4]),
              pullRequestID > 0 else {
            throw PRLocatorError.invalidPullRequestURL
        }

        organization = org
        project = parts[0]
        repository = parts[2]
        id = pullRequestID
    }

    private static func decodePathSegments(_ segments: [Substring]) -> [String]? {
        var decoded: [String] = []
        decoded.reserveCapacity(segments.count)
        for segment in segments {
            guard let value = String(segment).removingPercentEncoding,
                  !value.isEmpty,
                  !value.contains("/"),
                  !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
                return nil
            }
            decoded.append(value)
        }
        return decoded
    }
}
