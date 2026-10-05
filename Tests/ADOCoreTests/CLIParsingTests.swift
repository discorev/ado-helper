import XCTest
@testable import ADOCore

final class CLIParsingTests: XCTestCase {
    private let parser = CLIParser()

    func testParsesCommentWithExplicitReviewIdentity() throws {
        let command = try parser.parse([
            "pr", "comment", "42", "--profile", "work", "--file", "/Sources/App.swift",
            "--line", "8", "--end-line", "10", "--side", "right", "--body-file", "/tmp/body",
            "--commit", String(repeating: "a", count: 40), "--iteration", "3", "--change-id", "17"
        ])
        XCTAssertEqual(command, .prComment(target: .number(42), profile: "work", file: "/Sources/App.swift",
                                           line: 8, endLine: 10, side: "right", bodyFile: "/tmp/body",
                                           commit: String(repeating: "a", count: 40), iteration: 3, changeID: 17))
    }

    func testOmittedTargetMeansCurrentBranch() throws {
        XCTAssertEqual(try parser.parse(["pr", "show"]), .prShow(target: .currentBranch, profile: nil))
        XCTAssertEqual(try parser.parse(["pr", "clone", "--directory", "/tmp/review"]),
                       .prClone(target: .currentBranch, profile: nil, directory: "/tmp/review"))
    }

    func testRejectsImplicitCommentIdentityAndDuplicateOptions() {
        XCTAssertThrowsError(try parser.parse(["pr", "comment", "1", "--file", "a", "--line", "1",
                                               "--side", "right", "--body-file", "b"]))
        XCTAssertThrowsError(try parser.parse(["pr", "show", "1", "--profile", "a", "--profile", "b"]))
    }

    func testAuthGrammar() throws {
        XCTAssertEqual(try parser.parse(["auth", "add", "work", "--org", "https://dev.azure.com/acme", "--no-browser"]),
                       .authAdd(name: "work", organization: "https://dev.azure.com/acme", openBrowser: false))
        XCTAssertEqual(try parser.parse(["auth", "status"]), .authStatus(check: false))
        XCTAssertThrowsError(try parser.parse(["auth", "add", "work"]))
        XCTAssertThrowsError(try parser.parse(["auth", "add", "work", "--org", "a", "--org", "b"]))
    }
}
