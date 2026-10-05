import Foundation

public enum AuthError: LocalizedError, Equatable {
    case profileAlreadyExists(String)
    case organizationAlreadyConfigured(String)
    case invalidToken
    case invalidIdentity
    case identityChanged
    case cancelled
    case browserFailed
    case storageFailed
    case storageAndCleanupFailed
    case profileRemovalFailed(String)

    public var errorDescription: String? {
        switch self {
        case .profileAlreadyExists(let name):
            return "A profile named '\(name)' already exists. Use 'ado auth update \(name)' to rotate its token."
        case .organizationAlreadyConfigured(let organization):
            return "Azure DevOps organization '\(organization)' already has a profile. Update that profile instead."
        case .invalidToken:
            return "The token is not plausible. Paste the complete PAT without spaces or line breaks."
        case .invalidIdentity:
            return "Azure DevOps did not return a usable authenticated identity. No credentials were saved."
        case .identityChanged:
            return "The new token belongs to a different Azure DevOps identity. The existing token was not changed."
        case .cancelled:
            return "Authentication was cancelled; no credentials were saved."
        case .browserFailed:
            return "Could not open the Azure DevOps token settings page. Open the displayed URL in a browser and try again with --no-browser."
        case .storageFailed:
            return "Authentication succeeded, but the local profile could not be saved. No token was retained."
        case .storageAndCleanupFailed:
            return "The local profile could not be saved, and the new Keychain item could not be removed. No profile was created; remove the ado token in Keychain Access before retrying."
        case .profileRemovalFailed(let name):
            return "The Keychain token was removed, but local profile '\(name)' could not be removed. The profile remains without a token; retry 'ado auth remove \(name)'."
        }
    }
}

public struct AuthManager {
    private let loadProfiles: () throws -> [Profile]
    private let saveProfile: (Profile) throws -> Void
    private let removeProfile: (String) throws -> Void
    private let readToken: (Profile) throws -> String
    private let saveToken: (String, Profile) throws -> Void
    private let removeToken: (Profile) throws -> Void
    private let readSecret: (String) throws -> String
    private let confirm: (String) throws -> Bool
    private let requireInteractive: () throws -> Void
    private let openURL: (URL) throws -> Void
    private let validateIdentity: (Organization, String) async throws -> ADOIdentity
    private let output: (String) -> Void

    public init(store: ProfileStore, keychain: KeychainStore) {
        self.init(
            loadProfiles: store.load,
            saveProfile: store.save,
            removeProfile: store.remove,
            readToken: keychain.read,
            saveToken: keychain.save,
            removeToken: keychain.remove,
            readSecret: Terminal.readSecret,
            confirm: Terminal.confirm,
            requireInteractive: Terminal.requireInteractive,
            openURL: Self.openInDefaultBrowser,
            validateIdentity: { organization, token in
                try await ADOClient(organization: organization, token: token).identity()
            },
            output: Self.writeHumanOutput
        )
    }

    init(
        loadProfiles: @escaping () throws -> [Profile],
        saveProfile: @escaping (Profile) throws -> Void,
        removeProfile: @escaping (String) throws -> Void,
        readToken: @escaping (Profile) throws -> String,
        saveToken: @escaping (String, Profile) throws -> Void,
        removeToken: @escaping (Profile) throws -> Void,
        readSecret: @escaping (String) throws -> String,
        confirm: @escaping (String) throws -> Bool,
        requireInteractive: @escaping () throws -> Void,
        openURL: @escaping (URL) throws -> Void,
        validateIdentity: @escaping (Organization, String) async throws -> ADOIdentity,
        output: @escaping (String) -> Void
    ) {
        self.loadProfiles = loadProfiles
        self.saveProfile = saveProfile
        self.removeProfile = removeProfile
        self.readToken = readToken
        self.saveToken = saveToken
        self.removeToken = removeToken
        self.readSecret = readSecret
        self.confirm = confirm
        self.requireInteractive = requireInteractive
        self.openURL = openURL
        self.validateIdentity = validateIdentity
        self.output = output
    }

    public func add(name: String, organization: Organization, openBrowser: Bool = true) async throws {
        try ProfileStore.validateName(name)
        let profiles = try loadProfiles()
        guard !profiles.contains(where: { $0.name == name }) else {
            throw AuthError.profileAlreadyExists(name)
        }
        guard !profiles.contains(where: {
            $0.organization.name.caseInsensitiveCompare(organization.name) == .orderedSame
        }) else {
            throw AuthError.organizationAlreadyConfigured(organization.name)
        }

        try prepareForToken(organization: organization, openBrowser: openBrowser)
        var token = try readSecret("Azure DevOps PAT: ")
        defer { token = "" }
        try Self.validate(token: token)

        let identity = try await validateIdentity(organization, token)
        guard !identity.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw AuthError.invalidIdentity
        }
        let profile = Profile(name: name, organization: organization, identity: identity)
        let identityDescription = Self.identityDescription(identity)
        guard try confirm("Authenticated as \(identityDescription) for \(organization.name). Save this profile?") else {
            throw AuthError.cancelled
        }

        try saveToken(token, profile)
        do {
            try saveProfile(profile)
        } catch {
            do {
                try removeToken(profile)
            } catch {
                throw AuthError.storageAndCleanupFailed
            }
            throw AuthError.storageFailed
        }
        output("Saved profile '\(name)' for \(organization.url.absoluteString) as \(identityDescription).")
    }

    public func update(name: String, openBrowser: Bool = true) async throws {
        let profile = try profileNamed(name)
        try prepareForToken(organization: profile.organization, openBrowser: openBrowser)
        var token = try readSecret("New Azure DevOps PAT: ")
        defer { token = "" }
        try Self.validate(token: token)

        let identity = try await validateIdentity(profile.organization, token)
        guard !identity.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw AuthError.invalidIdentity
        }
        guard identity.id == profile.identity.id else {
            throw AuthError.identityChanged
        }
        let identityDescription = Self.identityDescription(identity)
        guard try confirm("Validated \(identityDescription) for \(profile.organization.name). Replace the stored token?") else {
            throw AuthError.cancelled
        }

        // The Keychain update is the only mutation. Validation and confirmation happen first,
        // so an invalid or rejected replacement leaves the prior item untouched.
        try saveToken(token, profile)
        output("Updated the Keychain token for profile '\(name)'.")
    }

    public func remove(name: String) throws {
        let profile = try profileNamed(name)
        // Delete the secret first. If this fails, the profile remains available for a safe retry.
        // KeychainStore.remove is idempotent, so retrying is also safe if profile removal fails.
        try removeToken(profile)
        do {
            try removeProfile(name)
        } catch {
            throw AuthError.profileRemovalFailed(name)
        }
        output("Removed local profile '\(name)' and its Keychain token. The PAT was not revoked in Azure DevOps.")
    }

    public static func patSettingsURL(for organization: Organization) -> URL {
        organization.url.appendingPathComponent("_usersSettings/tokens", isDirectory: false)
    }

    public static func scopeInstructions(for organization: Organization) -> String {
        """
        Create an organization-scoped, short-lived PAT for \(organization.name):
          Required: Code — Read (vso.code)
          Required: PR threads — Read & write (vso.threads_full; use "Show all scopes")
          Not needed by current ado commands: Build — Read (vso.build)
        Do not choose Full access or Code read/write. If "PR threads" is unavailable,
        check your organization policy with an administrator rather than broadening the token.
        Token settings: \(patSettingsURL(for: organization).absoluteString)
        """
    }

    static func validate(token: String) throws {
        guard (20...2048).contains(token.utf8.count),
              token == token.trimmingCharacters(in: .whitespacesAndNewlines),
              !token.unicodeScalars.contains(where: {
                  CharacterSet.controlCharacters.contains($0) || CharacterSet.whitespacesAndNewlines.contains($0)
              }) else {
            throw AuthError.invalidToken
        }
    }

    private func profileNamed(_ name: String) throws -> Profile {
        try ProfileStore.validateName(name)
        guard let profile = try loadProfiles().first(where: { $0.name == name }) else {
            throw ProfileStoreError.profileNotFound(name)
        }
        return profile
    }

    private func prepareForToken(organization: Organization, openBrowser: Bool) throws {
        try requireInteractive()
        output(Self.scopeInstructions(for: organization))
        if openBrowser {
            do {
                try openURL(Self.patSettingsURL(for: organization))
            } catch {
                throw AuthError.browserFailed
            }
        }
    }

    private static func terminalSafe(_ value: String) -> String {
        String(value.unicodeScalars.map { scalar in
            CharacterSet.controlCharacters.contains(scalar) ? "�" : Character(scalar)
        })
    }

    private static func identityDescription(_ identity: ADOIdentity) -> String {
        let displayName = terminalSafe(identity.displayName)
        let uniqueName = terminalSafe(identity.uniqueName)
        if displayName.isEmpty { return uniqueName }
        if uniqueName.isEmpty { return displayName }
        return "\(displayName) <\(uniqueName)>"
    }

    private static func openInDefaultBrowser(_ url: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        process.arguments = [url.absoluteString]
        try process.run()
        process.waitUntilExit()
        guard process.terminationReason == .exit, process.terminationStatus == 0 else {
            throw AuthError.browserFailed
        }
    }

    private static func writeHumanOutput(_ value: String) {
        guard let data = (value + "\n").data(using: .utf8) else { return }
        FileHandle.standardError.write(data)
    }
}
