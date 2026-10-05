import XCTest
@testable import ADOCore

final class IntegrationRegressionTests: XCTestCase {
    func testFullPRCommentUsesMergeBaseRatherThanFirstIterationAsLeftSide() async throws {
        let transport = IntegrationCaptureTransport()
        let client = ADOClient(organization: try Organization("example"), token: "TEST_ONLY_NOT_A_PAT", transport: transport)
        _ = try await client.createThread(project: "Project", repository: "Repo", id: 42,
            body: "Review finding", path: "/budget.py", startLine: 7, endLine: 7,
            side: "left", iteration: 4, changeTrackingID: 12)
        let request = await transport.request
        let data = try XCTUnwrap(request?.httpBody)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let context = try XCTUnwrap(payload["pullRequestThreadContext"] as? [String: Any])
        let iteration = try XCTUnwrap(context["iterationContext"] as? [String: Int])
        // ADO documents equal iterations as comparing the source against its common commit.
        XCTAssertEqual(iteration["firstComparingIteration"], 4)
        XCTAssertEqual(iteration["secondComparingIteration"], 4)
    }

    func testLegacySSHRemoteFormIsRecognised() throws {
        let remote = try AzureGitRemote.parse("ssh://fabrikam@vs-ssh.visualstudio.com:22/Billing-Platform/_git/Payments_API")
        XCTAssertEqual(remote.organization, "fabrikam")
        XCTAssertEqual(remote.project, "Billing-Platform")
        XCTAssertEqual(remote.repository, "Payments_API")
        XCTAssertNotNil(remote.sshURL)
    }

    func testDiffContainsActualChangesBetweenReviewedCommits() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let git = GitRunner()
        _ = try git.run(arguments: ["init"], directory: directory)
        let file = directory.appendingPathComponent("budget.txt")
        try "before\n".write(to: file, atomically: true, encoding: .utf8)
        _ = try git.run(arguments: ["add", "budget.txt"], directory: directory)
        let commit = ["-c", "user.name=ADO Test", "-c", "user.email=ado-test@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-m", "fixture"]
        _ = try git.run(arguments: commit, directory: directory)
        let base = try git.run(arguments: ["rev-parse", "HEAD"], directory: directory)
        try "after\n".write(to: file, atomically: true, encoding: .utf8)
        _ = try git.run(arguments: ["add", "budget.txt"], directory: directory)
        _ = try git.run(arguments: commit, directory: directory)
        let head = try git.run(arguments: ["rev-parse", "HEAD"], directory: directory)
        let result = ReviewCheckoutResult(directory: directory.path, sourceCommit: head, targetCommit: base, mergeBase: base)
        let diff = try ReviewCheckout().diff(result)
        XCTAssertTrue(diff.contains("-before"), diff)
        XCTAssertTrue(diff.contains("+after"), diff)
    }

    func testOrganisationIdentityIsCaseInsensitiveAcrossURLForms() throws {
        XCTAssertEqual(try Organization("FABRIKAM"), try Organization("https://fabrikam.visualstudio.com"))
        XCTAssertEqual(try Organization("https://dev.azure.com/Fabrikam/"), try Organization("fabrikam"))
    }

    func testPastedPRLinkMayIncludeFilesTabAndThreadSelection() throws {
        let locator = try PRLocator(url: "https://dev.azure.com/fabrikam/Billing-Platform/_git/Payments_API/pullrequest/4321?_a=files&path=%2Fmain.py#discussion-123")
        XCTAssertEqual(locator.organization.name, "fabrikam")
        XCTAssertEqual(locator.project, "Billing-Platform")
        XCTAssertEqual(locator.repository, "Payments_API")
        XCTAssertEqual(locator.id, 4321)
    }

    func testLegacyDefaultCollectionPRLinksResolve() throws {
        let locator = try PRLocator(url: "https://example.visualstudio.com/DefaultCollection/My%20Project/_git/Repo/pullrequest/42?_a=overview")
        XCTAssertEqual(locator.organization.name, "example")
        XCTAssertEqual(locator.project, "My Project")
        XCTAssertEqual(locator.id, 42)
    }
}

private actor IntegrationCaptureTransport: ADOHTTPTransport {
    var request: URLRequest?
    func data(for request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        self.request = request
        return (Data("{\"id\":1}".utf8), HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
    }
}
