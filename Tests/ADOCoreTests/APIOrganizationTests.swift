import XCTest
@testable import ADOCore

final class APIOrganizationTests: XCTestCase {
    func testOrganizationNormalizesSupportedForms() throws {
        let bare = try Organization("Example-Org")
        XCTAssertEqual(bare.name, "example-org")
        XCTAssertEqual(bare.url.absoluteString, "https://dev.azure.com/example-org")

        let modern = try Organization("https://dev.azure.com/example-org/")
        XCTAssertEqual(modern.name, "example-org")
        XCTAssertEqual(modern.url.absoluteString, "https://dev.azure.com/example-org")

        let legacy = try Organization("https://LegacyOrg.visualstudio.com/")
        XCTAssertEqual(legacy.name, "legacyorg")
        XCTAssertEqual(legacy.url.absoluteString, "https://dev.azure.com/legacyorg")
    }

    func testOrganizationRejectsAmbiguousOrUnsafeInputs() {
        let invalid = [
            "http://dev.azure.com/org",
            "https://user:password@dev.azure.com/org",
            "https://dev.azure.com:443/org",
            "https://dev.azure.com/org/project",
            "https://dev.azure.com/org?token=secret",
            "https://dev.azure.com/org#fragment",
            "https://sub.org.visualstudio.com",
            "bad/name",
            "-bad",
            "bad-",
            " bad"
        ]

        for value in invalid {
            XCTAssertThrowsError(try Organization(value), "Expected to reject \(value)")
        }
    }

    func testOrganizationCodableDecodingRevalidatesAndNormalizes() throws {
        let normalized = try JSONDecoder().decode(
            Organization.self,
            from: Data("{\"name\":\"FABRIKAM\"}".utf8)
        )
        XCTAssertEqual(normalized.name, "fabrikam")

        XCTAssertThrowsError(try JSONDecoder().decode(
            Organization.self,
            from: Data("{\"name\":\"https://evil.example/org\"}".utf8)
        ))
    }

    func testPRLocatorParsesModernAndLegacyURLs() throws {
        let modern = try PRLocator(
            url: "https://dev.azure.com/acme/My%20Project/_git/Repo/pullrequest/42"
        )
        XCTAssertEqual(modern.organization, try Organization("acme"))
        XCTAssertEqual(modern.project, "My Project")
        XCTAssertEqual(modern.repository, "Repo")
        XCTAssertEqual(modern.id, 42)

        let legacy = try PRLocator(
            url: "https://acme.visualstudio.com/Project/_git/Repo/pullrequest/7/"
        )
        XCTAssertEqual(legacy.organization, try Organization("acme"))
        XCTAssertEqual(legacy.project, "Project")
        XCTAssertEqual(legacy.repository, "Repo")
        XCTAssertEqual(legacy.id, 7)
    }

    func testPRLocatorRejectsNonPRAndEncodedPathSeparator() {
        let invalid = [
            "http://dev.azure.com/acme/Project/_git/Repo/pullrequest/1",
            "https://evil.example/acme/Project/_git/Repo/pullrequest/1",
            "https://dev.azure.com/acme/Project/_git/Repo/pullrequest/0",
            "https://dev.azure.com/acme/Project%2FExtra/_git/Repo/pullrequest/1"
        ]
        for value in invalid {
            XCTAssertThrowsError(try PRLocator(url: value), "Expected to reject \(value)")
        }
    }
}
