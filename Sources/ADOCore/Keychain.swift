import Foundation
import Security

public enum KeychainStoreError: LocalizedError, Equatable {
    case itemNotFound
    case invalidTokenData
    case accessConfiguration(OSStatus)
    case operation(String, OSStatus)

    public var errorDescription: String? {
        switch self {
        case .itemNotFound:
            return "No Keychain token was found for this profile. Run 'ado auth update NAME' for the affected profile."
        case .invalidTokenData:
            return "The token stored in Keychain is not valid text. Run 'ado auth update NAME' for the affected profile."
        case .accessConfiguration(let status):
            return "Could not restrict Keychain access to this executable (Keychain status \(status))."
        case .operation(let operation, let status):
            return "Keychain could not \(operation) the token (status \(status))."
        }
    }
}

public struct KeychainStore {
    static let service = "dev.ollies.ado-helper.pat"

    public init() {}

    public func read(profile: Profile) throws -> String {
        var query = baseQuery(profile: profile)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            throw KeychainStoreError.itemNotFound
        }
        guard status == errSecSuccess else {
            throw KeychainStoreError.operation("read", status)
        }
        guard let data = result as? Data,
              let token = String(data: data, encoding: .utf8),
              !token.isEmpty else {
            throw KeychainStoreError.invalidTokenData
        }
        return token
    }

    public func save(token: String, profile: Profile) throws {
        guard let tokenData = token.data(using: .utf8), !tokenData.isEmpty else {
            throw KeychainStoreError.invalidTokenData
        }
        let access = try executableOnlyAccess(profile: profile)
        let query = baseQuery(profile: profile)
        let attributes: [String: Any] = [
            kSecValueData as String: tokenData,
            kSecAttrAccess as String: access
        ]

        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var item = query
            item[kSecValueData as String] = tokenData
            item[kSecAttrAccess as String] = access
            item[kSecAttrLabel as String] = "ado token for \(profile.organization.name) (\(profile.name))"
            status = SecItemAdd(item as CFDictionary, nil)
            if status == errSecDuplicateItem {
                status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
            }
        }
        guard status == errSecSuccess else {
            throw KeychainStoreError.operation("save", status)
        }
    }

    public func remove(profile: Profile) throws {
        let status = SecItemDelete(baseQuery(profile: profile) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainStoreError.operation("remove", status)
        }
    }

    static func account(profile: Profile) -> String {
        // Length-prefixing prevents ambiguous collisions even if an identity ID contains punctuation.
        let organization = profile.organization.name.lowercased()
        return "\(organization.utf8.count):\(organization)\(profile.identity.id.utf8.count):\(profile.identity.id)"
    }

    private func baseQuery(profile: Profile) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: Self.account(profile: profile),
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any
        ]
    }

    private func executableOnlyAccess(profile: Profile) throws -> SecAccess {
        var trustedApplication: SecTrustedApplication?
        let trustedStatus = SecTrustedApplicationCreateFromPath(nil, &trustedApplication)
        guard trustedStatus == errSecSuccess, let trustedApplication else {
            throw KeychainStoreError.accessConfiguration(trustedStatus)
        }

        var access: SecAccess?
        let description = "ado access to \(profile.organization.name) token" as CFString
        let accessStatus = SecAccessCreate(description, [trustedApplication] as CFArray, &access)
        guard accessStatus == errSecSuccess, let access else {
            throw KeychainStoreError.accessConfiguration(accessStatus)
        }
        return access
    }
}
