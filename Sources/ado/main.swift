import ADOCore
import Darwin
import Foundation

@main
struct ADOCommandLine {
    static func main() async {
        do {
            let command = try CLIParser().parse(Array(CommandLine.arguments.dropFirst()))
            if command == .help {
                print(Application.help)
            } else {
                try await Application().run(command)
            }
        } catch {
            let message = terminalSafe(error.localizedDescription)
            FileHandle.standardError.write(Data("error: \(message)\n".utf8))
            Darwin.exit(1)
        }
    }

    private static func terminalSafe(_ value: String) -> String {
        String(value.unicodeScalars.map { CharacterSet.controlCharacters.contains($0) ? "�" : Character($0) })
    }
}

private struct Application {
    private let store: ProfileStore
    private let keychain = KeychainStore()

    init() throws { store = try ProfileStore() }

    func run(_ command: CLICommand) async throws {
        switch command {
        case .help:
            print(Self.help)
        case .authAdd(let name, let rawOrganization, let openBrowser):
            try await AuthManager(store: store, keychain: keychain)
                .add(name: name, organization: Organization(rawOrganization), openBrowser: openBrowser)
        case .authUpdate(let name, let openBrowser):
            try await AuthManager(store: store, keychain: keychain).update(name: name, openBrowser: openBrowser)
        case .authStatus(let check):
            try await authStatus(check: check)
        case .authRemove(let name):
            try AuthManager(store: store, keychain: keychain).remove(name: name)
        case .prShow(let target, let profile):
            let resolved = try await resolve(target, profileName: profile)
            try writeJSON(resolved.pullRequest)
        case .prThreads(let target, let profile):
            let resolved = try await resolve(target, profileName: profile)
            let repository = try repositoryIdentity(resolved.pullRequest, locator: resolved.locator)
            try writeJSON(try await resolved.client.threads(project: repository.project,
                                                            repository: repository.repository,
                                                            id: resolved.locator.id))
        case .prChanges(let target, let profile, let requestedIteration):
            let resolved = try await resolve(target, profileName: profile)
            let repository = try repositoryIdentity(resolved.pullRequest, locator: resolved.locator)
            let iterations = try await resolved.client.iterations(project: repository.project,
                                                                   repository: repository.repository,
                                                                   id: resolved.locator.id)
            let iteration = try requestedIteration ?? latestIteration(iterations)
            let changes = try await resolved.client.changes(project: repository.project,
                                                             repository: repository.repository,
                                                             id: resolved.locator.id, iteration: iteration)
            try writeJSON(["pullRequestId": resolved.locator.id, "iteration": iteration,
                           "changes": changes] as [String: Any])
        case .prClone(let target, let profile, let path):
            let resolved = try await resolve(target, profileName: profile)
            let result = try prepareCheckout(resolved, path: path)
            try writeEncodable(result)
        case .prDiff(let target, let profile, let path):
            let resolved = try await resolve(target, profileName: profile)
            let checkout = ReviewCheckout()
            let result = try prepareCheckout(resolved, path: path, checkout: checkout)
            let diff = try checkout.diff(result)
            try writeJSON(["checkout": ["directory": result.directory, "sourceCommit": result.sourceCommit,
                                         "targetCommit": result.targetCommit, "mergeBase": result.mergeBase],
                           "diff": diff])
        case .prComment(let target, let profile, let file, let line, let endLine,
                        let side, let bodyFile, let commit, let iteration, let changeID):
            try await comment(target: target, profileName: profile, file: file, line: line,
                              endLine: endLine, side: side, bodyFile: bodyFile, commit: commit,
                              iteration: iteration, changeID: changeID)
        }
    }

    private func authStatus(check: Bool) async throws {
        let profiles = try store.load()
        if profiles.isEmpty { print("No authentication profiles configured."); return }
        for profile in profiles {
            if check {
                let token = try keychain.read(profile: profile)
                let identity = try await ADOClient(organization: profile.organization, token: token).identity()
                guard identity.id == profile.identity.id else {
                    throw CLIParseError.message("Profile '\(profile.name)' authenticated as a different identity.")
                }
                print("\(profile.name)\t\(profile.organization.url.absoluteString)\t\(terminalSafe(identity.uniqueName))\tverified")
            } else {
                print("\(profile.name)\t\(profile.organization.url.absoluteString)\t\(terminalSafe(profile.identity.uniqueName))")
            }
        }
    }

    private struct Resolved {
        let locator: PRLocator
        let profile: Profile
        let client: ADOClient
        let pullRequest: [String: Any]
    }

    private func resolve(_ target: PullRequestTarget, profileName: String?) async throws -> Resolved {
        let initial: (PRLocator, Profile)
        switch target {
        case .url(let raw):
            let locator = try PRLocator(url: raw)
            let profile = try selectedProfile(profileName, organization: locator.organization)
            initial = (locator, profile)
        case .number(let id):
            if let profileName {
                let profile = try store.profile(name: profileName)
                initial = (PRLocator(organization: profile.organization, project: nil, repository: nil, id: id), profile)
            } else {
                let context = try LocalGitContext.discover()
                let organization = try Organization(context.remote.organization)
                initial = (PRLocator(organization: organization, project: nil, repository: nil, id: id),
                           try store.profile(organization: organization))
            }
        case .currentBranch:
            let context = try LocalGitContext.discover()
            let organization = try Organization(context.remote.organization)
            let profile = try selectedProfile(profileName, organization: organization)
            let client = ADOClient(organization: organization, token: try keychain.read(profile: profile))
            let candidates = try await client.listPullRequests(project: context.remote.project,
                                                            repository: context.remote.repository,
                                                            sourceBranch: "refs/heads/\(context.branch)")
            // The endpoint searches the target repository. A fork can have an unrelated
            // branch with the same name; require a PR sourced from this repository.
            let matches = candidates.filter {
                $0["forkSource"] == nil || $0["forkSource"] is NSNull
            }.filter { ($0["sourceRefName"] as? String) == "refs/heads/\(context.branch)" }
            guard matches.count == 1, let id = integer(matches[0]["pullRequestId"]), id > 0 else {
                if matches.isEmpty { throw CLIParseError.message("No active pull request was found for the current branch.") }
                throw CLIParseError.message("More than one pull request matched the current branch; pass a PR URL or number.")
            }
            let locator = PRLocator(organization: organization, project: context.remote.project,
                                    repository: context.remote.repository, id: id)
            // List responses can truncate descriptions. Always retrieve the full PR below.
            initial = (locator, profile)
        }
        let client = ADOClient(organization: initial.0.organization, token: try keychain.read(profile: initial.1))
        let pullRequest = try await client.pullRequest(id: initial.0.id)
        try validatePullRequest(pullRequest, locator: initial.0)
        return Resolved(locator: initial.0, profile: initial.1, client: client, pullRequest: pullRequest)
    }

    private func selectedProfile(_ name: String?, organization: Organization) throws -> Profile {
        let profile = try name.map(store.profile(name:)) ?? store.profile(organization: organization)
        guard profile.organization.name.caseInsensitiveCompare(organization.name) == .orderedSame else {
            throw CLIParseError.message("Profile '\(profile.name)' belongs to \(profile.organization.name), not \(organization.name).")
        }
        return profile
    }

    private func validatePullRequest(_ value: [String: Any], locator: PRLocator) throws {
        guard integer(value["pullRequestId"]) == locator.id else {
            throw CLIParseError.message("Azure DevOps returned a different pull request ID.")
        }
        let repository = try repositoryIdentity(value, locator: locator)
        if let expected = locator.project,
           !repository.projectAliases.contains(where: { $0.caseInsensitiveCompare(expected) == .orderedSame }) {
            throw CLIParseError.message("Pull request project does not match the target URL or repository.")
        }
        if let expected = locator.repository,
           !repository.repositoryAliases.contains(where: { $0.caseInsensitiveCompare(expected) == .orderedSame }) {
            throw CLIParseError.message("Pull request repository does not match the target URL or repository.")
        }
    }

    private func repositoryIdentity(_ value: [String: Any], locator: PRLocator) throws
        -> (project: String, repository: String, projectAliases: [String], repositoryAliases: [String]) {
        guard let repository = value["repository"] as? [String: Any],
              let repositoryName = repository["name"] as? String,
              let project = repository["project"] as? [String: Any],
              let projectName = project["name"] as? String,
              !repositoryName.isEmpty, !projectName.isEmpty else { throw GitError.responseMissing("repository project and name") }
        return (projectName, repositoryName,
                [projectName, project["id"] as? String].compactMap { $0 },
                [repositoryName, repository["id"] as? String].compactMap { $0 })
    }

    private func prepareCheckout(_ resolved: Resolved, path: String?, checkout: ReviewCheckout = ReviewCheckout()) throws -> ReviewCheckoutResult {
        let info = try PullRequestGitInfo(pullRequest: resolved.pullRequest)
        guard info.id == resolved.locator.id,
              info.target.organization.caseInsensitiveCompare(resolved.locator.organization.name) == .orderedSame else {
            throw GitError.invalidRemote("pull request repository does not match its organization")
        }
        return try checkout.prepare(info, directory: path.map { URL(fileURLWithPath: $0).standardizedFileURL })
    }

    private func comment(target: PullRequestTarget, profileName: String?, file: String, line: Int,
                         endLine: Int, side: String, bodyFile: String, commit: String,
                         iteration: Int, changeID: Int) async throws {
        let resolved = try await resolve(target, profileName: profileName)
        let repository = try repositoryIdentity(resolved.pullRequest, locator: resolved.locator)
        let canonicalPath = try commentPath(file)
        let body = try readCommentBody(bodyFile)
        let postingIdentity = try await resolved.client.identity()
        guard postingIdentity.id == resolved.profile.identity.id else {
            throw CLIParseError.message("The saved token authenticated as a different account. No comment was posted; run 'ado auth update \(resolved.profile.name)'.")
        }
        try verifySourceCommit(resolved.pullRequest, equals: commit)
        let beforeIterations = try await resolved.client.iterations(project: repository.project,
                                                                    repository: repository.repository,
                                                                    id: resolved.locator.id)
        guard try latestIteration(beforeIterations) == iteration else {
            throw CLIParseError.message("The requested iteration is no longer current; refresh changes and retry.")
        }
        let changes = try await resolved.client.changes(project: repository.project,
                                                         repository: repository.repository,
                                                         id: resolved.locator.id, iteration: iteration)
        guard let selectedChange = changes.first(where: { change in
            integer(change["changeTrackingId"]) == changeID && changePath(change, side: side) == canonicalPath
        }) else {
            throw CLIParseError.message("The change ID does not identify the requested file in this iteration.")
        }
        try CommentAnchor.validateSide(side, changeType: selectedChange["changeType"])
        // Re-read both identities after the potentially paginated changes request. These are the
        // final reads before the one non-idempotent operation.
        let readyPullRequest = try await resolved.client.pullRequest(id: resolved.locator.id)
        try verifySourceCommit(readyPullRequest, equals: commit)
        let readyIterations = try await resolved.client.iterations(project: repository.project,
                                                                   repository: repository.repository,
                                                                   id: resolved.locator.id)
        guard try latestIteration(readyIterations) == iteration else {
            throw CLIParseError.message("The requested iteration changed while preparing the comment; refresh changes and retry.")
        }
        let anchorIteration = try iterationCommits(readyIterations, id: iteration,
                                                   expectedSourceCommit: commit,
                                                   requireCommonCommit: side == "left")
        let anchorRepository = try anchorRepository(side: side, pullRequest: readyPullRequest,
                                                    target: repository,
                                                    organization: resolved.locator.organization)
        guard let anchorPath = changePath(selectedChange, side: side) else {
            throw CLIParseError.message("Azure DevOps did not return a path for the requested comment side.")
        }
        let anchorCommit = side == "right" ? anchorIteration.source : anchorIteration.common!
        let content = try await resolved.client.fileContent(project: anchorRepository.project,
                                                            repository: anchorRepository.repository,
                                                            path: anchorPath, commit: anchorCommit)
        try CommentAnchor.validate(side: side, startLine: line, endLine: endLine,
                                   content: content, changeType: selectedChange["changeType"])

        // File retrieval is another network round trip. Re-read the PR and iteration once more so
        // the source and comparison commits used for the anchor are still current at POST time.
        let finalPullRequest = try await resolved.client.pullRequest(id: resolved.locator.id)
        try verifySourceCommit(finalPullRequest, equals: commit)
        let finalIterations = try await resolved.client.iterations(project: repository.project,
                                                                   repository: repository.repository,
                                                                   id: resolved.locator.id)
        guard try latestIteration(finalIterations) == iteration,
              try iterationCommits(finalIterations, id: iteration,
                                   expectedSourceCommit: commit,
                                   requireCommonCommit: side == "left") == anchorIteration else {
            throw CLIParseError.message("The reviewed commits changed while validating the comment anchor; refresh changes and retry.")
        }
        let created: [String: Any]
        do {
            created = try await resolved.client.createThread(project: repository.project,
                                                              repository: repository.repository,
                                                              id: resolved.locator.id, body: body, path: canonicalPath,
                                                              startLine: line, endLine: endLine, side: side,
                                                              iteration: iteration, changeTrackingID: changeID)
        } catch {
            throw CLIParseError.message("Comment submission was not confirmed: \(error.localizedDescription) Inspect PR threads before retrying; the request may have reached Azure DevOps.")
        }
        guard let threadID = integer(created["id"]), threadID > 0 else {
            throw CommentCreatedError(threadID: nil, detail: "Azure DevOps did not return a thread ID")
        }
        do {
            try validateThread(created, id: threadID, path: canonicalPath, line: line, endLine: endLine, side: side)
            let afterPullRequest = try await resolved.client.pullRequest(id: resolved.locator.id)
            try verifySourceCommit(afterPullRequest, equals: commit)
            let afterIterations = try await resolved.client.iterations(project: repository.project,
                                                                       repository: repository.repository,
                                                                       id: resolved.locator.id)
            guard try latestIteration(afterIterations) == iteration else {
                throw CLIParseError.message("the pull request source or iteration changed during creation")
            }
            let readback = try await resolved.client.threads(project: repository.project,
                                                             repository: repository.repository,
                                                             id: resolved.locator.id)
            guard let thread = readback.first(where: { integer($0["id"]) == threadID }) else {
                throw CLIParseError.message("the new thread was absent from the immediate readback")
            }
            try validateThread(thread, id: threadID, path: canonicalPath, line: line, endLine: endLine, side: side)
        } catch {
            throw CommentCreatedError(threadID: threadID, detail: error.localizedDescription)
        }
        try writeJSON(created)
    }

    private func verifySourceCommit(_ pullRequest: [String: Any], equals expected: String) throws {
        guard let object = pullRequest["lastMergeSourceCommit"] as? [String: Any],
              let actual = object["commitId"] as? String, GitValidation.isCommitSHA(actual) else {
            throw GitError.responseMissing("reviewed source commit")
        }
        guard actual.caseInsensitiveCompare(expected) == .orderedSame else {
            throw CLIParseError.message("The pull request source commit does not match --commit; refresh the review and retry.")
        }
    }

    private func latestIteration(_ iterations: [[String: Any]]) throws -> Int {
        guard let latest = iterations.compactMap({ integer($0["id"]) }).max(), latest > 0 else {
            throw GitError.responseMissing("pull request iteration")
        }
        return latest
    }

    private struct IterationCommits: Equatable {
        let source: String
        let common: String?
    }

    private func iterationCommits(_ iterations: [[String: Any]], id: Int,
                                  expectedSourceCommit: String,
                                  requireCommonCommit: Bool) throws -> IterationCommits {
        guard let value = iterations.first(where: { integer($0["id"]) == id }),
              let sourceObject = value["sourceRefCommit"] as? [String: Any],
              let source = sourceObject["commitId"] as? String,
              GitValidation.isCommitSHA(source) else {
            throw GitError.responseMissing("iteration sourceRefCommit")
        }
        guard source.caseInsensitiveCompare(expectedSourceCommit) == .orderedSame else {
            throw CLIParseError.message("The iteration source commit does not match --commit; refresh changes and retry.")
        }
        var common: String?
        if requireCommonCommit {
            guard let commonObject = value["commonRefCommit"] as? [String: Any],
                  let value = commonObject["commitId"] as? String,
                  GitValidation.isCommitSHA(value) else {
                throw GitError.responseMissing("iteration commonRefCommit")
            }
            common = value.lowercased()
        }
        return IterationCommits(source: source.lowercased(), common: common)
    }

    private func anchorRepository(
        side: String,
        pullRequest: [String: Any],
        target: (project: String, repository: String, projectAliases: [String], repositoryAliases: [String]),
        organization: Organization
    ) throws -> (project: String, repository: String) {
        guard side == "right" else { return (target.project, target.repository) }
        guard let rawFork = pullRequest["forkSource"], !(rawFork is NSNull) else {
            return (target.project, target.repository)
        }
        guard let fork = rawFork as? [String: Any],
              let repository = fork["repository"] as? [String: Any],
              let project = repository["project"] as? [String: Any],
              let sshURL = repository["sshUrl"] as? String,
              let projectValue = (project["id"] as? String) ?? (project["name"] as? String),
              let repositoryValue = (repository["id"] as? String) ?? (repository["name"] as? String) else {
            throw CLIParseError.message("The fork source repository metadata is unavailable; no comment was posted.")
        }
        let remote: AzureGitRemote
        do { remote = try AzureGitRemote.validatedSSHURL(sshURL) }
        catch { throw CLIParseError.message("The fork source repository SSH metadata is not a trusted Azure URL; no comment was posted.") }
        guard remote.organization.caseInsensitiveCompare(organization.name) == .orderedSame else {
            throw CLIParseError.message("Cross-organization fork comments are not supported.")
        }
        let projectAliases = [project["id"] as? String, project["name"] as? String].compactMap { $0 }
        let repositoryAliases = [repository["id"] as? String, repository["name"] as? String].compactMap { $0 }
        guard projectAliases.contains(where: { $0.caseInsensitiveCompare(remote.project) == .orderedSame }),
              repositoryAliases.contains(where: { $0.caseInsensitiveCompare(remote.repository) == .orderedSame }) else {
            throw CLIParseError.message("The fork source repository metadata does not match its Azure SSH URL; no comment was posted.")
        }
        return (projectValue, repositoryValue)
    }

    private func commentPath(_ raw: String) throws -> String {
        let path = raw.hasPrefix("/") ? raw : "/" + raw
        guard path.count > 1, !path.contains("\u{0000}"), !path.contains("\n"), !path.contains("\r") else {
            throw CLIParseError.message("--file must be a nonempty repository path.")
        }
        return path
    }

    private func changePath(_ change: [String: Any], side: String) -> String? {
        let path: String?
        if side == "left", let original = change["originalPath"] as? String { path = original }
        else { path = (change["item"] as? [String: Any])?["path"] as? String }
        guard let path else { return nil }
        return path.hasPrefix("/") ? path : "/" + path
    }

    private func validateThread(_ thread: [String: Any], id: Int, path: String,
                                line: Int, endLine: Int, side: String) throws {
        guard integer(thread["id"]) == id,
              let context = thread["threadContext"] as? [String: Any],
              (context["filePath"] as? String) == path,
              let start = context["\(side)FileStart"] as? [String: Any],
              let end = context["\(side)FileEnd"] as? [String: Any],
              integer(start["line"]) == line, integer(end["line"]) == endLine else {
            throw CLIParseError.message("thread readback did not match the requested file and line context")
        }
    }

    private func readCommentBody(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: path)
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        guard let size = attributes[.size] as? NSNumber, size.intValue <= 1_048_576 else {
            throw CLIParseError.message("Comment body file must be 1 MiB or smaller.")
        }
        let data = try Data(contentsOf: url)
        guard let body = String(data: data, encoding: .utf8), !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw CLIParseError.message("Comment body file must contain nonempty UTF-8 text.")
        }
        return body
    }

    private func terminalSafe(_ value: String) -> String {
        String(value.unicodeScalars.map { CharacterSet.controlCharacters.contains($0) ? "�" : Character($0) })
    }

    private func integer(_ value: Any?) -> Int? {
        if let value = value as? Int { return value }
        if let value = value as? NSNumber { return value.intValue }
        return nil
    }

    private func writeJSON(_ object: Any) throws {
        guard JSONSerialization.isValidJSONObject(object) else { throw ADOClientError.invalidResponse }
        let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }

    private func writeEncodable<T: Encodable>(_ value: T) throws {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        FileHandle.standardOutput.write(try encoder.encode(value)); FileHandle.standardOutput.write(Data("\n".utf8))
    }

    static let help = """
    Usage:
      ado auth add NAME --org URL [--no-browser]
      ado auth update NAME [--no-browser]
      ado auth status [--check]
      ado auth remove NAME
      ado pr show [TARGET] [--profile NAME]
      ado pr threads [TARGET] [--profile NAME]
      ado pr changes [TARGET] [--profile NAME] [--iteration N]
      ado pr clone [TARGET] [--profile NAME] [--directory PATH]
      ado pr diff [TARGET] [--profile NAME] [--directory PATH]
      ado pr comment [TARGET] --file PATH --line N [--end-line N] --side left|right
          --body-file PATH --commit SHA --iteration N --change-id N [--profile NAME]

    TARGET is an Azure DevOps PR URL or positive PR number. Omit it to use the current branch.
    Pull-request data is written as JSON; authentication and help are human-readable.
    """
}

private struct CommentCreatedError: LocalizedError {
    let threadID: Int?
    let detail: String
    var errorDescription: String? {
        if let threadID {
            return "Inline comment thread \(threadID) was created, but verification failed: \(detail). Do not retry automatically; inspect that thread first."
        }
        return "The inline comment was created, but Azure DevOps returned no thread ID. Do not retry automatically; inspect the pull request threads first."
    }
}
