import Darwin
import XCTest
@testable import ADOCore

final class AuthStorageTests: XCTestCase {
    func testProfileStoreRoundTripsWithoutWritingATokenAndUsesOwnerOnlyPermissions() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try ProfileStore(directory: directory)
        let profile = try makeProfile(name: "work", organization: "example", identityID: "user-1")

        try store.save(profile)

        XCTAssertEqual(try store.load(), [profile])
        XCTAssertEqual(try store.profile(name: "work"), profile)
        XCTAssertEqual(try store.profile(organization: Organization("example")), profile)

        let data = try Data(contentsOf: directory.appendingPathComponent("profiles.json"))
        let text = try XCTUnwrap(String(data: data, encoding: .utf8))
        XCTAssertFalse(text.contains("secret-token"))
        XCTAssertEqual(try permissions(of: directory), 0o700)
        XCTAssertEqual(try permissions(of: directory.appendingPathComponent("profiles.json")), 0o600)
    }

    func testProfileStoreRejectsDuplicateOrganization() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try ProfileStore(directory: directory)
        try store.save(try makeProfile(name: "first", organization: "example", identityID: "user-1"))

        XCTAssertThrowsError(
            try store.save(makeProfile(name: "second", organization: "example", identityID: "user-2"))
        ) { error in
            XCTAssertEqual(error as? ProfileStoreError, .duplicateOrganization("example"))
        }
    }

    func testProfileStoreValidatesNamesBeforeUsingThem() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try ProfileStore(directory: directory)

        for name in ["", ".hidden", "../escape", "has space", String(repeating: "a", count: 65)] {
            XCTAssertThrowsError(
                try store.save(makeProfile(name: name, organization: "example", identityID: "user-1")),
                "Expected invalid name: \(name)"
            ) { error in
                guard case .invalidName = error as? ProfileStoreError else {
                    return XCTFail("Unexpected error: \(error)")
                }
            }
        }
    }

    func testProfileStoreRejectsOrganizationThatBypassedOrganizationInitializer() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try ProfileStore(directory: directory)
        let data = Data(#"[{"identity":{"displayName":"Test","id":"user-1","uniqueName":"test@example.com"},"name":"work","organization":{"name":"../not-an-org"}}]"#.utf8)
        let profilesURL = directory.appendingPathComponent("profiles.json")
        try data.write(to: profilesURL)
        XCTAssertEqual(chmod(profilesURL.path, 0o600), 0)

        XCTAssertThrowsError(try store.load()) { error in
            XCTAssertEqual(error as? ProfileStoreError, .invalidProfileFile)
        }
    }

    func testKeychainAccountSeparatesOrganizationAndIdentity() throws {
        let first = try makeProfile(name: "one", organization: "alpha", identityID: "same-id")
        let second = try makeProfile(name: "two", organization: "beta", identityID: "same-id")
        let third = try makeProfile(name: "three", organization: "alpha", identityID: "other-id")

        XCTAssertNotEqual(KeychainStore.account(profile: first), KeychainStore.account(profile: second))
        XCTAssertNotEqual(KeychainStore.account(profile: first), KeychainStore.account(profile: third))
        XCTAssertTrue(KeychainStore.account(profile: first).contains("alpha"))
        XCTAssertTrue(KeychainStore.account(profile: first).contains("same-id"))
    }

    private func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("ado-auth-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
        return url
    }

    private func makeProfile(name: String, organization: String, identityID: String) throws -> Profile {
        Profile(
            name: name,
            organization: try Organization(organization),
            identity: ADOIdentity(id: identityID, displayName: "Test User", uniqueName: "test@example.com")
        )
    }

    private func permissions(of url: URL) throws -> Int {
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        return try XCTUnwrap(attributes[.posixPermissions] as? NSNumber).intValue
    }
}
