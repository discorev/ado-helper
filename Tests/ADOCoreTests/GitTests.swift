import Foundation
import XCTest
@testable import ADOCore

final class GitTests: XCTestCase {
    func testParsesSupportedAzureRemotes() throws {
        XCTAssertEqual(try AzureGitRemote.parse("https://dev.azure.com/acme/My%20Project/_git/service"),
                       try AzureGitRemote(organization: "acme", project: "My Project", repository: "service"))
        let modern = try AzureGitRemote.parse("git@ssh.dev.azure.com:v3/acme/My%20Project/service")
        XCTAssertEqual(modern.organization, "acme")
        XCTAssertEqual(modern.project, "My Project")
        XCTAssertEqual(modern.sshURL, "git@ssh.dev.azure.com:v3/acme/My%20Project/service")
        XCTAssertEqual(try AzureGitRemote.parse("acme@vs-ssh.visualstudio.com:v3/acme/project/repo").repository, "repo")
    }

    func testRejectsUntrustedAndMismatchedRemotes() throws {
        XCTAssertThrowsError(try AzureGitRemote.parse("git@github.com:acme/repo.git"))
        XCTAssertThrowsError(try AzureGitRemote.parse("https://user@example.com/org/project/_git/repo"))
        XCTAssertThrowsError(try AzureGitRemote.parse("https://dev.azure.com/acme//project/_git/repo"))
        XCTAssertThrowsError(try AzureGitRemote.parse("other@vs-ssh.visualstudio.com:v3/acme/project/repo"))
        let expected = try AzureGitRemote(organization: "acme", project: "p", repository: "r")
        XCTAssertThrowsError(try AzureGitRemote.validatedSSHURL("git@ssh.dev.azure.com:v3/acme/p/other", expected: expected))
    }

    func testPullRequestInfoRejectsArbitrarySSHURL() {
        let value: [String: Any] = [
            "pullRequestId": 7,
            "repository": ["name": "repo", "sshUrl": "git@attacker.test:repo", "project": ["name": "project"]],
            "sourceRefName": "refs/heads/topic", "targetRefName": "refs/heads/main",
            "lastMergeSourceCommit": ["commitId": String(repeating: "a", count: 40)],
            "lastMergeTargetCommit": ["commitId": String(repeating: "b", count: 40)]
        ]
        XCTAssertThrowsError(try PullRequestGitInfo(pullRequest: value))
    }

    func testCheckoutUsesArgumentArraysAndProtectsExistingDirectory() throws {
        let temp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: temp) }
        try FileManager.default.createDirectory(at: temp, withIntermediateDirectories: true)
        let occupied = temp.appendingPathComponent("occupied")
        try FileManager.default.createDirectory(at: occupied, withIntermediateDirectories: false)
        let runner = RecordingGitRunner()
        let checkout = ReviewCheckout(runner: runner)
        XCTAssertThrowsError(try checkout.prepare(try fixture(), directory: occupied))
        XCTAssertTrue(runner.calls.isEmpty)

        let destination = temp.appendingPathComponent("managed")
        runner.destinationToCreate = destination
        let result = try checkout.prepare(try fixture(), directory: destination)
        XCTAssertEqual(result.sourceCommit, String(repeating: "a", count: 40))
        XCTAssertEqual(runner.calls[0].arguments,
                       ["-c", "core.hooksPath=/dev/null", "clone", "--no-checkout",
                        "git@ssh.dev.azure.com:v3/acme/project/repo", destination.path])
        XCTAssertTrue(FileManager.default.fileExists(atPath: destination.appendingPathComponent(".git/ado-review.json").path))
        XCTAssertTrue(runner.calls.contains { $0.arguments == ["-c", "core.hooksPath=/dev/null", "fetch", "--no-tags", "git@ssh.dev.azure.com:v3/acme/project/repo", "refs/heads/topic"] })
        XCTAssertTrue(runner.calls.contains { $0.arguments == ["-c", "core.hooksPath=/dev/null", "checkout", "--detach", String(repeating: "a", count: 40)] })
    }

    func testManagedCheckoutRejectsDirtyWorkingTree() throws {
        let temp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: temp) }
        let runner = RecordingGitRunner()
        runner.destinationToCreate = temp
        let checkout = ReviewCheckout(runner: runner)
        _ = try checkout.prepare(try fixture(), directory: temp)
        runner.statusOutput = " M Sources/App.swift"
        XCTAssertThrowsError(try checkout.prepare(try fixture(), directory: temp))
    }

    func testLocalContextPrefersConfiguredUpstreamAndRejectsAmbiguousRemotes() throws {
        let values = [
            "rev-parse --abbrev-ref HEAD": "topic",
            "remote": "origin\nupstream",
            "remote get-url origin": "git@ssh.dev.azure.com:v3/acme/project/fork",
            "remote get-url upstream": "git@ssh.dev.azure.com:v3/acme/project/main",
            "config --get branch.topic.remote": "upstream"
        ]
        let selected = try LocalGitContext.discover(runner: DictionaryGitRunner(values: values))
        XCTAssertEqual(selected.remote.repository, "main")
        var ambiguous = values
        ambiguous.removeValue(forKey: "config --get branch.topic.remote")
        XCTAssertThrowsError(try LocalGitContext.discover(runner: DictionaryGitRunner(values: ambiguous)))
    }

    func testGitRunnerDrainsLargeOutputWithoutDeadlock() throws {
        let temp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: temp) }
        try FileManager.default.createDirectory(at: temp, withIntermediateDirectories: true)
        let git = GitRunner()
        _ = try git.run(arguments: ["init"], directory: temp)
        let file = temp.appendingPathComponent("large.txt")
        let before = (0..<80_000).map { "before-\($0)" }.joined(separator: "\n") + "\n"
        try before.write(to: file, atomically: true, encoding: .utf8)
        _ = try git.run(arguments: ["add", "large.txt"], directory: temp)
        _ = try git.run(arguments: ["-c", "user.name=ADO Test", "-c", "user.email=test@example.invalid",
                                    "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                                    "commit", "-m", "fixture"], directory: temp)
        let after = before.replacingOccurrences(of: "before-", with: "after-")
        try after.write(to: file, atomically: true, encoding: .utf8)
        let output = try git.run(arguments: ["diff", "--no-ext-diff", "--no-textconv"], directory: temp)
        XCTAssertGreaterThan(output.utf8.count, 1_000_000)
    }

    private func fixture() throws -> PullRequestGitInfo {
        let remote = try AzureGitRemote.validatedSSHURL("git@ssh.dev.azure.com:v3/acme/project/repo")
        return PullRequestGitInfo(id: 7, target: remote, targetSSHURL: remote.sshURL!,
                                  source: remote, sourceSSHURL: remote.sshURL!, sourceRef: "refs/heads/topic",
                                  targetRef: "refs/heads/main", sourceCommit: String(repeating: "a", count: 40),
                                  targetCommit: String(repeating: "b", count: 40))
    }
}

private final class RecordingGitRunner: GitRunning {
    struct Call { let arguments: [String]; let directory: URL? }
    var calls: [Call] = []
    var destinationToCreate: URL?
    private var fetchCount = 0

    func run(arguments: [String], directory: URL?) throws -> String {
        calls.append(Call(arguments: arguments, directory: directory))
        if arguments.contains("clone"), let destinationToCreate {
            try FileManager.default.createDirectory(at: destinationToCreate.appendingPathComponent(".git"), withIntermediateDirectories: true)
            return ""
        }
        if arguments.contains("fetch") { fetchCount += 1; return "" }
        if arguments.contains("status") { return statusOutput }
        if arguments == ["rev-parse", "FETCH_HEAD"] {
            return fetchCount == 1 ? String(repeating: "b", count: 40) : String(repeating: "a", count: 40)
        }
        if arguments == ["rev-parse", "HEAD"] { return String(repeating: "a", count: 40) }
        if arguments.first == "merge-base" { return String(repeating: "c", count: 40) }
        return ""
    }
    var statusOutput = ""
}

private struct DictionaryGitRunner: GitRunning {
    let values: [String: String]
    func run(arguments: [String], directory: URL?) throws -> String {
        let key = arguments.joined(separator: " ")
        guard let value = values[key] else { throw GitError.commandFailed("missing mock") }
        return value
    }
}
