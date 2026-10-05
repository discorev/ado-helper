import XCTest
@testable import ADOCore

final class CommentAnchorTests: XCTestCase {
    func testAdditionAndUndeleteOnlyHaveRightSide() throws {
        XCTAssertThrowsError(try CommentAnchor.validate(side: "left", startLine: 1, endLine: 1,
                                                        content: "new\n", changeType: "add"))
        XCTAssertNoThrow(try CommentAnchor.validate(side: "right", startLine: 1, endLine: 1,
                                                    content: "new\n", changeType: "add, edit"))
        XCTAssertThrowsError(try CommentAnchor.validate(side: "left", startLine: 1, endLine: 1,
                                                        content: "restored", changeType: "undelete, delete"))
        XCTAssertNoThrow(try CommentAnchor.validate(side: "right", startLine: 1, endLine: 1,
                                                    content: "restored", changeType: "undelete"))
    }

    func testDeletionOnlyHasLeftSide() throws {
        XCTAssertThrowsError(try CommentAnchor.validate(side: "right", startLine: 1, endLine: 1,
                                                        content: "old", changeType: "delete"))
        XCTAssertNoThrow(try CommentAnchor.validate(side: "left", startLine: 1, endLine: 1,
                                                    content: "old", changeType: "delete"))
    }

    func testNumericAzureChangeFlagsAreSupported() throws {
        XCTAssertThrowsError(try CommentAnchor.validateSide("left", changeType: NSNumber(value: 1)))
        XCTAssertThrowsError(try CommentAnchor.validateSide("right", changeType: NSNumber(value: 16)))
        XCTAssertNoThrow(try CommentAnchor.validateSide("right", changeType: NSNumber(value: 32)))
        XCTAssertThrowsError(try CommentAnchor.validateSide("left", changeType: NSNumber(value: 32)))
    }

    func testEditedFilesHaveBothSidesAndMissingMetadataFailsClosed() throws {
        XCTAssertNoThrow(try CommentAnchor.validateSide("left", changeType: "edit"))
        XCTAssertNoThrow(try CommentAnchor.validateSide("right", changeType: "edit"))
        XCTAssertThrowsError(try CommentAnchor.validateSide("right", changeType: nil))
    }

    func testLineCountDoesNotInventLineAfterTerminalNewline() throws {
        XCTAssertEqual(CommentAnchor.lineCount(in: ""), 0)
        XCTAssertEqual(CommentAnchor.lineCount(in: "one"), 1)
        XCTAssertEqual(CommentAnchor.lineCount(in: "one\n"), 1)
        XCTAssertEqual(CommentAnchor.lineCount(in: "one\ntwo"), 2)
        XCTAssertEqual(CommentAnchor.lineCount(in: "one\ntwo\n"), 2)
        XCTAssertNoThrow(try CommentAnchor.validate(side: "right", startLine: 1, endLine: 2,
                                                    content: "one\ntwo\n", changeType: "edit"))
        XCTAssertThrowsError(try CommentAnchor.validate(side: "right", startLine: 2, endLine: 3,
                                                        content: "one\ntwo\n", changeType: "edit"))
    }

    func testEmptyFileHasNoValidPositiveAnchor() {
        XCTAssertThrowsError(try CommentAnchor.validate(side: "right", startLine: 1, endLine: 1,
                                                        content: "", changeType: "add"))
    }
}
