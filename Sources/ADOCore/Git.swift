import Foundation

public enum GitError: LocalizedError {
    case invalidRemote(String)
    case unsafeValue(String)
    case commandFailed(String)
    case notRepository
    case managedDirectoryRequired(String)
    case responseMissing(String)

    public var errorDescription: String? {
        switch self {
        case .invalidRemote: return "Unsupported, mismatched, or unsafe Azure Git remote."
        case .unsafeValue(let value): return "Unsafe Git value: \(value)"
        case .commandFailed(let message): return message
        case .notRepository: return "No Git repository was found. Supply --profile with a PR number, or use a PR URL."
        case .managedDirectoryRequired(let path): return "Refusing to use \(path): it is not an ado-managed review directory for this pull request."
        case .responseMissing(let field): return "Azure DevOps pull request response is missing \(field)."
        }
    }
}

public enum GitValidation {
    public static func isCommitSHA(_ value: String) -> Bool {
        value.count == 40 && value.unicodeScalars.allSatisfy {
            (48...57).contains($0.value) || (65...70).contains($0.value) || (97...102).contains($0.value)
        }
    }

    public static func validateRef(_ value: String) throws {
        guard value.hasPrefix("refs/heads/"), value.count > "refs/heads/".count,
              !value.contains(".."), !value.contains("@{"), !value.contains("\\"),
              !value.contains("//"), !value.hasSuffix("/"), !value.hasSuffix("."), value != "@",
              value.unicodeScalars.allSatisfy({ $0.value > 32 && $0.value != 127 }),
              value.rangeOfCharacter(from: CharacterSet(charactersIn: "~^:?*[")) == nil,
              !value.split(separator: "/").contains(where: { $0.hasPrefix(".") || $0.hasSuffix(".lock") }) else {
            throw GitError.unsafeValue("invalid branch ref")
        }
    }
}

public struct AzureGitRemote: Equatable, Codable {
    public let organization: String
    public let project: String
    public let repository: String
    public let sshURL: String?

    public init(organization: String, project: String, repository: String, sshURL: String? = nil) throws {
        self.organization = try Self.component(organization, label: "organization")
        self.project = try Self.component(project, label: "project")
        self.repository = try Self.component(repository, label: "repository")
        if let sshURL {
            let parsed = try Self.parse(sshURL)
            guard parsed.organization.caseInsensitiveCompare(self.organization) == .orderedSame,
                  parsed.project == self.project, parsed.repository == self.repository else {
                throw GitError.invalidRemote("SSH URL does not identify the expected repository")
            }
            self.sshURL = sshURL
        } else {
            self.sshURL = nil
        }
    }

    public static func parse(_ raw: String) throws -> AzureGitRemote {
        if raw.hasPrefix("git@ssh.dev.azure.com:v3/") {
            return try parseSCP(raw, prefix: "git@ssh.dev.azure.com:v3/")
        }
        if let at = raw.firstIndex(of: "@") {
            let user = String(raw[..<at])
            let rest = String(raw[raw.index(after: at)...])
            if rest.hasPrefix("vs-ssh.visualstudio.com:v3/") {
                let result = try parseSCP(raw, prefix: "\(user)@vs-ssh.visualstudio.com:v3/")
                guard user.caseInsensitiveCompare(result.organization) == .orderedSame else {
                    throw GitError.invalidRemote("legacy SSH user must match organization")
                }
                return result
            }
        }
        if let components = URLComponents(string: raw), components.scheme?.lowercased() == "ssh",
           components.password == nil, components.query == nil, components.fragment == nil,
           (components.port == nil || components.port == 22),
           !components.percentEncodedPath.dropFirst().contains("//") {
            let host = components.host?.lowercased()
            let parts = try components.percentEncodedPath.split(separator: "/", omittingEmptySubsequences: true)
                .map { try decode(String($0)) }
            if host == "vs-ssh.visualstudio.com", let user = components.user,
               parts.count == 3, parts[1].lowercased() == "_git" {
                let base = try AzureGitRemote(organization: user, project: parts[0], repository: parts[2])
                return try AzureGitRemote(organization: base.organization, project: base.project,
                                          repository: base.repository, uncheckedSSHURL: raw)
            }
            if host == "ssh.dev.azure.com", components.user == "git", parts.count == 4,
               parts[0].lowercased() == "v3" {
                let base = try AzureGitRemote(organization: parts[1], project: parts[2], repository: parts[3])
                return try AzureGitRemote(organization: base.organization, project: base.project,
                                          repository: base.repository, uncheckedSSHURL: raw)
            }
            throw GitError.invalidRemote(raw)
        }
        guard let url = URL(string: raw), url.scheme?.lowercased() == "https",
              url.user == nil, url.password == nil, url.port == nil,
              url.query == nil, url.fragment == nil,
              !url.path.dropFirst().contains("//") else { throw GitError.invalidRemote(raw) }
        let parts = try decodedPath(url)
        let host = url.host?.lowercased()
        if host == "dev.azure.com", parts.count == 4, parts[2].lowercased() == "_git" {
            return try AzureGitRemote(organization: parts[0], project: parts[1], repository: parts[3])
        }
        if let host, host.hasSuffix(".visualstudio.com"), !host.hasPrefix("."),
           parts.count == 3, parts[1].lowercased() == "_git" {
            let org = String(host.dropLast(".visualstudio.com".count))
            guard !org.contains(".") else { throw GitError.invalidRemote(raw) }
            return try AzureGitRemote(organization: org, project: parts[0], repository: parts[2])
        }
        throw GitError.invalidRemote(raw)
    }

    public static func validatedSSHURL(_ raw: String, expected: AzureGitRemote? = nil) throws -> AzureGitRemote {
        let parsed = try parse(raw)
        guard parsed.sshURL != nil else { throw GitError.invalidRemote("expected an Azure SSH URL") }
        if let expected {
            guard parsed.organization.caseInsensitiveCompare(expected.organization) == .orderedSame,
                  parsed.project == expected.project, parsed.repository == expected.repository else {
                throw GitError.invalidRemote("SSH URL does not identify the expected repository")
            }
        }
        return parsed
    }

    private static func parseSCP(_ raw: String, prefix: String) throws -> AzureGitRemote {
        let tail = String(raw.dropFirst(prefix.count))
        guard !tail.contains(":"), !tail.contains("?"), !tail.contains("#") else { throw GitError.invalidRemote(raw) }
        let encoded = tail.split(separator: "/", omittingEmptySubsequences: false)
        guard encoded.count == 3 else { throw GitError.invalidRemote(raw) }
        let parts = try encoded.map { try decode(String($0)) }
        let base = try AzureGitRemote(organization: parts[0], project: parts[1], repository: parts[2])
        return try AzureGitRemote(organization: base.organization, project: base.project,
                                  repository: base.repository, uncheckedSSHURL: raw)
    }

    private init(organization: String, project: String, repository: String, uncheckedSSHURL: String) throws {
        self.organization = try Self.component(organization, label: "organization")
        self.project = try Self.component(project, label: "project")
        self.repository = try Self.component(repository, label: "repository")
        self.sshURL = uncheckedSSHURL
    }

    private static func decodedPath(_ url: URL) throws -> [String] {
        try url.path.split(separator: "/", omittingEmptySubsequences: true).map { try decode(String($0)) }
    }

    private static func decode(_ value: String) throws -> String {
        guard let decoded = value.removingPercentEncoding, !decoded.isEmpty else { throw GitError.invalidRemote("invalid path encoding") }
        return decoded
    }

    private static func component(_ value: String, label: String) throws -> String {
        let forbidden = CharacterSet(charactersIn: "/\\:\u{0000}\n\r")
        guard !value.isEmpty, value != ".", value != "..", value.rangeOfCharacter(from: forbidden) == nil else {
            throw GitError.unsafeValue("invalid \(label)")
        }
        return value
    }
}

public protocol GitRunning {
    func run(arguments: [String], directory: URL?) throws -> String
}

public struct GitRunner: GitRunning {
    public init() {}
    public func run(arguments: [String], directory: URL? = nil) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = arguments
        process.currentDirectoryURL = directory
        let stdout = Pipe(), stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr
        do { try process.run() } catch { throw GitError.commandFailed("Unable to start Git: \(error.localizedDescription)") }
        let stdoutData = DataBox(), stderrData = DataBox()
        let readers = DispatchGroup()
        readers.enter()
        DispatchQueue.global().async {
            stdoutData.value = stdout.fileHandleForReading.readDataToEndOfFile()
            readers.leave()
        }
        readers.enter()
        DispatchQueue.global().async {
            stderrData.value = stderr.fileHandleForReading.readDataToEndOfFile()
            readers.leave()
        }
        process.waitUntilExit()
        readers.wait()
        let output = String(decoding: stdoutData.value, as: UTF8.self)
        let diagnostic = String(decoding: stderrData.value, as: UTF8.self)
        guard process.terminationStatus == 0 else {
            let safe = diagnostic.trimmingCharacters(in: .whitespacesAndNewlines)
            throw GitError.commandFailed(safe.isEmpty ? "Git command failed." : "Git failed: \(safe)")
        }
        return output.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

private final class DataBox: @unchecked Sendable {
    var value = Data()
}

public struct LocalGitContext {
    public let remote: AzureGitRemote
    public let branch: String

    public static func discover(runner: GitRunning = GitRunner(), directory: URL? = nil) throws -> LocalGitContext {
        let branch: String
        do {
            branch = try runner.run(arguments: ["rev-parse", "--abbrev-ref", "HEAD"], directory: directory)
        } catch { throw GitError.notRepository }
        guard !branch.isEmpty, branch != "HEAD" else { throw GitError.commandFailed("The current repository is in detached HEAD state.") }
        let remoteNames = (try? runner.run(arguments: ["remote"], directory: directory))?.split(separator: "\n").map(String.init) ?? []
        var supported: [(name: String, remote: AzureGitRemote)] = []
        for name in remoteNames {
            if let url = try? runner.run(arguments: ["remote", "get-url", name], directory: directory),
               let parsed = try? AzureGitRemote.parse(url) { supported.append((name, parsed)) }
        }
        if let upstream = try? runner.run(arguments: ["config", "--get", "branch.\(branch).remote"], directory: directory),
           upstream != "." {
            guard let match = supported.first(where: { $0.name == upstream }) else {
                throw GitError.invalidRemote("configured upstream is not a supported Azure repository")
            }
            return LocalGitContext(remote: match.remote, branch: branch)
        }
        let identities = Set(supported.map {
            "\($0.remote.organization.lowercased())\u{0000}\($0.remote.project.lowercased())\u{0000}\($0.remote.repository.lowercased())"
        })
        guard identities.count <= 1 else {
            throw GitError.invalidRemote("multiple Azure remotes identify different repositories; configure the branch upstream")
        }
        if let preferred = supported.first(where: { $0.name == "origin" }) ?? supported.first {
            return LocalGitContext(remote: preferred.remote, branch: branch)
        }
        throw GitError.invalidRemote("no supported Azure remote found")
    }
}

public struct PullRequestGitInfo: Equatable {
    public let id: Int
    public let target: AzureGitRemote
    public let targetSSHURL: String
    public let source: AzureGitRemote
    public let sourceSSHURL: String
    public let sourceRef: String
    public let targetRef: String
    public let sourceCommit: String
    public let targetCommit: String
}

public extension PullRequestGitInfo {
    init(pullRequest value: [String: Any]) throws {
        guard let id = value["pullRequestId"] as? Int else { throw GitError.responseMissing("pullRequestId") }
        guard let repository = value["repository"] as? [String: Any] else { throw GitError.responseMissing("repository") }
        func repoInfo(_ repo: [String: Any]) throws -> (AzureGitRemote, String) {
            guard let project = repo["project"] as? [String: Any], let projectName = project["name"] as? String,
                  let repoName = repo["name"] as? String, let ssh = repo["sshUrl"] as? String else {
                throw GitError.responseMissing("repository project/name/sshUrl")
            }
            let remote = try AzureGitRemote.validatedSSHURL(ssh)
            let projectValues = [projectName, project["id"] as? String].compactMap { $0 }
            let repositoryValues = [repoName, repo["id"] as? String].compactMap { $0 }
            guard projectValues.contains(where: { $0.caseInsensitiveCompare(remote.project) == .orderedSame }),
                  repositoryValues.contains(where: { $0.caseInsensitiveCompare(remote.repository) == .orderedSame }) else {
                throw GitError.invalidRemote("API repository metadata does not match its SSH URL")
            }
            return (remote, ssh)
        }
        let targetPair = try repoInfo(repository)
        let sourceRepo = ((value["forkSource"] as? [String: Any])?["repository"] as? [String: Any]) ?? repository
        let sourcePair = try repoInfo(sourceRepo)
        guard targetPair.0.organization.caseInsensitiveCompare(sourcePair.0.organization) == .orderedSame else {
            throw GitError.invalidRemote("cross-organization forks are not supported")
        }
        guard let sourceRef = value["sourceRefName"] as? String, let targetRef = value["targetRefName"] as? String,
              let sourceObject = value["lastMergeSourceCommit"] as? [String: Any],
              let sourceCommit = sourceObject["commitId"] as? String,
              let targetObject = value["lastMergeTargetCommit"] as? [String: Any],
              let targetCommit = targetObject["commitId"] as? String else { throw GitError.responseMissing("source/target refs and commits") }
        try GitValidation.validateRef(sourceRef); try GitValidation.validateRef(targetRef)
        guard GitValidation.isCommitSHA(sourceCommit), GitValidation.isCommitSHA(targetCommit) else { throw GitError.unsafeValue("invalid commit SHA from API") }
        self.init(id: id, target: targetPair.0, targetSSHURL: targetPair.1,
                  source: sourcePair.0, sourceSSHURL: sourcePair.1, sourceRef: sourceRef,
                  targetRef: targetRef, sourceCommit: sourceCommit.lowercased(), targetCommit: targetCommit.lowercased())
    }
}

public struct ReviewCheckoutResult: Codable, Equatable {
    public let directory: String
    public let sourceCommit: String
    public let targetCommit: String
    public let mergeBase: String
}

public struct ReviewCheckout {
    private let runner: GitRunning
    private let fileManager: FileManager
    public init(runner: GitRunning = GitRunner(), fileManager: FileManager = .default) {
        self.runner = runner; self.fileManager = fileManager
    }

    public func prepare(_ info: PullRequestGitInfo, directory requested: URL? = nil) throws -> ReviewCheckoutResult {
        let directory = requested ?? defaultDirectory(for: info)
        let marker = directory.appendingPathComponent(".git", isDirectory: true).appendingPathComponent("ado-review.json")
        if fileManager.fileExists(atPath: directory.path) {
            guard let data = try? Data(contentsOf: marker), let existing = try? JSONDecoder().decode(Marker.self, from: data),
                  existing == Marker(info) else { throw GitError.managedDirectoryRequired(directory.path) }
            let status = try runner.run(arguments: ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false",
                                                    "status", "--porcelain", "--untracked-files=all"], directory: directory)
            guard status.isEmpty else {
                throw GitError.commandFailed("Managed review checkout has local changes. Commit, move, or remove them before refreshing it.")
            }
        } else {
            try fileManager.createDirectory(at: directory.deletingLastPathComponent(), withIntermediateDirectories: true)
            _ = try runner.run(arguments: ["-c", "core.hooksPath=/dev/null", "clone", "--no-checkout",
                                           info.targetSSHURL, directory.path], directory: nil)
            let data = try JSONEncoder().encode(Marker(info))
            try data.write(to: marker, options: [.atomic])
        }
        try fetch(url: info.targetSSHURL, ref: info.targetRef, expected: info.targetCommit, directory: directory)
        try fetch(url: info.sourceSSHURL, ref: info.sourceRef, expected: info.sourceCommit, directory: directory)
        _ = try runner.run(arguments: ["-c", "core.hooksPath=/dev/null", "checkout", "--detach", info.sourceCommit], directory: directory)
        let head = try runner.run(arguments: ["rev-parse", "HEAD"], directory: directory).lowercased()
        guard head == info.sourceCommit else { throw GitError.commandFailed("Checked out commit does not match the reviewed source commit.") }
        let mergeBase = try runner.run(arguments: ["merge-base", info.targetCommit, info.sourceCommit], directory: directory)
        guard GitValidation.isCommitSHA(mergeBase) else { throw GitError.commandFailed("Git returned an invalid merge base.") }
        return ReviewCheckoutResult(directory: directory.path, sourceCommit: info.sourceCommit,
                                    targetCommit: info.targetCommit, mergeBase: mergeBase.lowercased())
    }

    public func diff(_ result: ReviewCheckoutResult) throws -> String {
        try runner.run(arguments: ["diff", "--no-ext-diff", "--no-textconv", result.mergeBase, result.sourceCommit, "--"],
                       directory: URL(fileURLWithPath: result.directory))
    }

    private func fetch(url: String, ref: String, expected: String, directory: URL) throws {
        _ = try runner.run(arguments: ["-c", "core.hooksPath=/dev/null", "fetch", "--no-tags", url, ref], directory: directory)
        let fetched = try runner.run(arguments: ["rev-parse", "FETCH_HEAD"], directory: directory).lowercased()
        guard fetched == expected else { throw GitError.commandFailed("Fetched ref no longer matches the reviewed commit. Refresh the pull request and retry.") }
    }

    private func defaultDirectory(for info: PullRequestGitInfo) -> URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Developer/ado-reviews", isDirectory: true)
            .appendingPathComponent(info.target.organization, isDirectory: true)
            .appendingPathComponent("\(info.target.repository)-pr-\(info.id)", isDirectory: true)
    }

    private struct Marker: Codable, Equatable {
        let organization: String; let project: String; let repository: String; let pullRequestID: Int
        init(_ info: PullRequestGitInfo) {
            organization = info.target.organization; project = info.target.project
            repository = info.target.repository; pullRequestID = info.id
        }
    }
}
