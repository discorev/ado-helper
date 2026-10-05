import XCTest
@testable import ADOCore

final class AuthManagerTests: XCTestCase {
    private final class State {
        var profiles: [Profile] = []
        var savedProfiles: [Profile] = []
        var tokenWrites: [(String, Profile)] = []
        var removedTokens: [Profile] = []
        var openedURLs: [URL] = []
        var messages: [String] = []
        var confirmationPrompts: [String] = []
        var validationCalls = 0
        var interactiveChecks = 0
        var cleanupAttempts = 0
        var profileRemovalAttempts = 0
    }

    private struct ExpectedFailure: Error {}

    func testAddOpensOrganizationPATPageValidatesAndSavesAfterConfirmation() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Example User", uniqueName: "user@example.com")
        let token = String(repeating: "a", count: 52)
        let manager = makeManager(state: state, token: token, identityResult: .success(identity))

        try await manager.add(name: "work", organization: organization)

        XCTAssertEqual(state.openedURLs, [URL(string: "https://dev.azure.com/example/_usersSettings/tokens")!])
        XCTAssertEqual(state.validationCalls, 1)
        XCTAssertEqual(state.tokenWrites.map(\.0), [token])
        XCTAssertEqual(state.savedProfiles.map(\.name), ["work"])
        XCTAssertEqual(state.tokenWrites.first?.1.identity, identity)
        XCTAssertEqual(state.confirmationPrompts.count, 1)
        XCTAssertTrue(state.confirmationPrompts[0].contains("Example User"))
        XCTAssertTrue(state.confirmationPrompts[0].contains("user@example.com"))
        XCTAssertTrue(state.messages.joined(separator: "\n").contains("vso.threads_full"))
        XCTAssertFalse(state.messages.joined(separator: "\n").contains(token))
    }

    func testInvalidTokenNeverCallsNetworkOrStorage() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Example User", uniqueName: "user@example.com")
        let manager = makeManager(state: state, token: "short", identityResult: .success(identity))

        do {
            try await manager.add(name: "work", organization: organization, openBrowser: false)
            XCTFail("Expected invalid token")
        } catch {
            XCTAssertEqual(error as? AuthError, .invalidToken)
        }
        XCTAssertEqual(state.validationCalls, 0)
        XCTAssertTrue(state.tokenWrites.isEmpty)
        XCTAssertTrue(state.savedProfiles.isEmpty)
        XCTAssertTrue(state.openedURLs.isEmpty)
    }

    func testUpdateWithDifferentIdentityLeavesOldTokenUntouched() async throws {
        let state = State()
        let existing = try profile(identityID: "identity-1")
        state.profiles = [existing]
        let otherIdentity = ADOIdentity(id: "identity-2", displayName: "Other User", uniqueName: "other@example.com")
        let manager = makeManager(
            state: state,
            token: String(repeating: "b", count: 52),
            identityResult: .success(otherIdentity)
        )

        do {
            try await manager.update(name: "work", openBrowser: false)
            XCTFail("Expected identity mismatch")
        } catch {
            XCTAssertEqual(error as? AuthError, .identityChanged)
        }
        XCTAssertTrue(state.tokenWrites.isEmpty)
        XCTAssertTrue(state.confirmationPrompts.isEmpty)
    }

    func testValidationFailureLeavesOldTokenUntouched() async throws {
        let state = State()
        state.profiles = [try profile(identityID: "identity-1")]
        let manager = makeManager(
            state: state,
            token: String(repeating: "c", count: 52),
            identityResult: .failure(ExpectedFailure())
        )

        do {
            try await manager.update(name: "work", openBrowser: false)
            XCTFail("Expected validation error")
        } catch is ExpectedFailure {
            // Expected.
        }
        XCTAssertTrue(state.tokenWrites.isEmpty)
        XCTAssertTrue(state.confirmationPrompts.isEmpty)
    }

    func testAddRollsBackKeychainItemWhenProfileSaveFails() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Example User", uniqueName: "user@example.com")
        let token = String(repeating: "d", count: 52)
        let manager = makeManager(
            state: state,
            token: token,
            identityResult: .success(identity),
            profileSaveFails: true
        )

        do {
            try await manager.add(name: "work", organization: organization, openBrowser: false)
            XCTFail("Expected storage failure")
        } catch {
            XCTAssertEqual(error as? AuthError, .storageFailed)
        }
        XCTAssertEqual(state.tokenWrites.count, 1)
        XCTAssertEqual(state.removedTokens.map(\.name), ["work"])
    }

    func testCancelledConfirmationDoesNotSaveProfileOrToken() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Example User", uniqueName: "user@example.com")
        let manager = makeManager(
            state: state,
            token: String(repeating: "e", count: 52),
            identityResult: .success(identity),
            confirmResult: false
        )

        do {
            try await manager.add(name: "work", organization: organization, openBrowser: false)
            XCTFail("Expected cancellation")
        } catch {
            XCTAssertEqual(error as? AuthError, .cancelled)
        }
        XCTAssertEqual(state.validationCalls, 1)
        XCTAssertTrue(state.tokenWrites.isEmpty)
        XCTAssertTrue(state.savedProfiles.isEmpty)
    }

    func testIdentityConfirmationIncludesDisplayNameAndUniqueName() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Alex Smith", uniqueName: "alex.one@example.com")
        let manager = makeManager(
            state: state,
            token: String(repeating: "f", count: 52),
            identityResult: .success(identity)
        )

        try await manager.add(name: "work", organization: organization, openBrowser: false)

        let prompt = try XCTUnwrap(state.confirmationPrompts.first)
        XCTAssertTrue(prompt.contains("Alex Smith <alex.one@example.com>"))
    }

    func testNoninteractiveSessionFailsBeforeOpeningBrowser() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Example User", uniqueName: "user@example.com")
        let manager = makeManager(
            state: state,
            token: String(repeating: "g", count: 52),
            identityResult: .success(identity),
            interactiveFails: true
        )

        do {
            try await manager.add(name: "work", organization: organization)
            XCTFail("Expected interactive terminal failure")
        } catch {
            XCTAssertEqual(error as? TerminalError, .noInteractiveTerminal)
        }
        XCTAssertEqual(state.interactiveChecks, 1)
        XCTAssertTrue(state.openedURLs.isEmpty)
        XCTAssertEqual(state.validationCalls, 0)
    }

    func testProfileSaveAndCleanupFailureReportsPossibleRetainedKeychainItem() async throws {
        let state = State()
        let organization = try Organization("example")
        let identity = ADOIdentity(id: "identity-1", displayName: "Example User", uniqueName: "user@example.com")
        let manager = makeManager(
            state: state,
            token: String(repeating: "h", count: 52),
            identityResult: .success(identity),
            profileSaveFails: true,
            tokenCleanupFails: true
        )

        do {
            try await manager.add(name: "work", organization: organization, openBrowser: false)
            XCTFail("Expected cleanup failure")
        } catch {
            XCTAssertEqual(error as? AuthError, .storageAndCleanupFailed)
            XCTAssertTrue(error.localizedDescription.contains("Keychain"))
        }
        XCTAssertEqual(state.cleanupAttempts, 1)
        XCTAssertEqual(state.tokenWrites.count, 1)
    }

    func testRemoveKeychainFailurePreservesProfileForRetry() throws {
        let state = State()
        let existing = try profile(identityID: "identity-1")
        state.profiles = [existing]
        let manager = makeManager(
            state: state,
            token: String(repeating: "i", count: 52),
            identityResult: .success(existing.identity),
            tokenCleanupFails: true
        )

        XCTAssertThrowsError(try manager.remove(name: "work")) { error in
            XCTAssertTrue(error is ExpectedFailure)
        }
        XCTAssertEqual(state.profiles, [existing])
        XCTAssertEqual(state.cleanupAttempts, 1)
        XCTAssertEqual(state.profileRemovalAttempts, 0)
    }

    func testRemoveProfileFailureReportsThatTokenIsGoneAndProfileRemains() throws {
        let state = State()
        let existing = try profile(identityID: "identity-1")
        state.profiles = [existing]
        let manager = makeManager(
            state: state,
            token: String(repeating: "j", count: 52),
            identityResult: .success(existing.identity),
            profileRemovalFails: true
        )

        XCTAssertThrowsError(try manager.remove(name: "work")) { error in
            XCTAssertEqual(error as? AuthError, .profileRemovalFailed("work"))
            XCTAssertTrue(error.localizedDescription.contains("Keychain token was removed"))
            XCTAssertTrue(error.localizedDescription.contains("profile remains without a token"))
        }
        XCTAssertEqual(state.removedTokens, [existing])
        XCTAssertEqual(state.profiles, [existing])
        XCTAssertEqual(state.profileRemovalAttempts, 1)
    }

    func testScopeGuidanceUsesLeastPrivilegeScopesAndOrgSpecificURL() throws {
        let organization = try Organization("example")
        let guidance = AuthManager.scopeInstructions(for: organization)

        XCTAssertTrue(guidance.contains("vso.code"))
        XCTAssertTrue(guidance.contains("vso.threads_full"))
        XCTAssertTrue(guidance.contains("vso.build"))
        XCTAssertTrue(guidance.contains("Do not choose Full access"))
        XCTAssertFalse(guidance.contains("vso.code_write"))
        XCTAssertTrue(guidance.contains("organization policy"))
        XCTAssertEqual(
            AuthManager.patSettingsURL(for: organization).absoluteString,
            "https://dev.azure.com/example/_usersSettings/tokens"
        )
    }

    private func profile(identityID: String) throws -> Profile {
        Profile(
            name: "work",
            organization: try Organization("example"),
            identity: ADOIdentity(id: identityID, displayName: "Example User", uniqueName: "user@example.com")
        )
    }

    private func makeManager(
        state: State,
        token: String,
        identityResult: Result<ADOIdentity, Error>,
        profileSaveFails: Bool = false,
        tokenCleanupFails: Bool = false,
        confirmResult: Bool = true,
        interactiveFails: Bool = false,
        profileRemovalFails: Bool = false
    ) -> AuthManager {
        AuthManager(
            loadProfiles: { state.profiles },
            saveProfile: { profile in
                if profileSaveFails { throw ExpectedFailure() }
                state.savedProfiles.append(profile)
                if let index = state.profiles.firstIndex(where: { $0.name == profile.name }) {
                    state.profiles[index] = profile
                } else {
                    state.profiles.append(profile)
                }
            },
            removeProfile: { name in
                state.profileRemovalAttempts += 1
                if profileRemovalFails { throw ExpectedFailure() }
                state.profiles.removeAll { $0.name == name }
            },
            readToken: { _ in token },
            saveToken: { savedToken, profile in state.tokenWrites.append((savedToken, profile)) },
            removeToken: { profile in
                state.cleanupAttempts += 1
                if tokenCleanupFails { throw ExpectedFailure() }
                state.removedTokens.append(profile)
            },
            readSecret: { _ in token },
            confirm: { prompt in
                state.confirmationPrompts.append(prompt)
                return confirmResult
            },
            requireInteractive: {
                state.interactiveChecks += 1
                if interactiveFails { throw TerminalError.noInteractiveTerminal }
            },
            openURL: { url in state.openedURLs.append(url) },
            validateIdentity: { _, _ in
                state.validationCalls += 1
                return try identityResult.get()
            },
            output: { message in state.messages.append(message) }
        )
    }
}
