import Darwin
import Foundation

public struct Profile: Codable, Equatable {
    public let name: String
    public let organization: Organization
    public let identity: ADOIdentity

    public init(name: String, organization: Organization, identity: ADOIdentity) {
        self.name = name
        self.organization = organization
        self.identity = identity
    }
}

public enum ProfileStoreError: LocalizedError, Equatable {
    case invalidName(String)
    case invalidDirectory(String)
    case invalidProfileFile
    case duplicateName(String)
    case duplicateOrganization(String)
    case profileNotFound(String)
    case organizationNotConfigured(String)
    case io(String)

    public var errorDescription: String? {
        switch self {
        case .invalidName(let name):
            return "Invalid profile name '\(name)'. Use 1-64 letters, numbers, dots, underscores, or hyphens, starting with a letter or number."
        case .invalidDirectory(let path):
            return "The profile directory is not a safe directory: \(path)"
        case .invalidProfileFile:
            return "The profile file is invalid or contains duplicate profiles."
        case .duplicateName(let name):
            return "A profile named '\(name)' already exists."
        case .duplicateOrganization(let organization):
            return "A profile already exists for Azure DevOps organization '\(organization)'."
        case .profileNotFound(let name):
            return "Profile '\(name)' was not found."
        case .organizationNotConfigured(let name):
            return "No profile is configured for '\(name)'. Run 'ado auth add NAME --org https://dev.azure.com/\(name)' in an interactive terminal."
        case .io(let operation):
            return "Could not \(operation) the local profile store."
        }
    }
}

public struct ProfileStore {
    public let directory: URL
    private let profilesURL: URL

    public init(directory: URL? = nil) throws {
        let selectedDirectory = directory
            ?? FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent(".config", isDirectory: true)
                .appendingPathComponent("ado", isDirectory: true)
        self.directory = selectedDirectory.standardizedFileURL
        self.profilesURL = self.directory.appendingPathComponent("profiles.json", isDirectory: false)
        try Self.prepareDirectory(self.directory)
    }

    public func load() throws -> [Profile] {
        guard FileManager.default.fileExists(atPath: profilesURL.path) else {
            return []
        }

        var info = stat()
        let status = profilesURL.path.withCString { lstat($0, &info) }
        guard status == 0,
              (info.st_mode & S_IFMT) == S_IFREG,
              info.st_uid == getuid(),
              (info.st_mode & 0o077) == 0 else {
            throw ProfileStoreError.invalidProfileFile
        }

        let data: Data
        do {
            data = try Data(contentsOf: profilesURL, options: [.mappedIfSafe])
        } catch {
            throw ProfileStoreError.io("read")
        }

        let profiles: [Profile]
        do {
            profiles = try JSONDecoder().decode([Profile].self, from: data)
        } catch {
            throw ProfileStoreError.invalidProfileFile
        }
        try Self.validate(profiles)
        return profiles.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    public func save(_ profile: Profile) throws {
        try Self.validateName(profile.name)
        guard !profile.identity.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw ProfileStoreError.invalidProfileFile
        }

        var profiles = try load()
        if let conflicting = profiles.first(where: {
            $0.organization.name.caseInsensitiveCompare(profile.organization.name) == .orderedSame
                && $0.name != profile.name
        }) {
            throw ProfileStoreError.duplicateOrganization(conflicting.organization.name)
        }

        if let index = profiles.firstIndex(where: { $0.name == profile.name }) {
            profiles[index] = profile
        } else {
            profiles.append(profile)
        }
        try Self.validate(profiles)
        try write(profiles.sorted { $0.name < $1.name })
    }

    public func remove(name: String) throws {
        try Self.validateName(name)
        var profiles = try load()
        guard let index = profiles.firstIndex(where: { $0.name == name }) else {
            throw ProfileStoreError.profileNotFound(name)
        }
        profiles.remove(at: index)
        try write(profiles)
    }

    public func profile(name: String) throws -> Profile {
        try Self.validateName(name)
        guard let profile = try load().first(where: { $0.name == name }) else {
            throw ProfileStoreError.profileNotFound(name)
        }
        return profile
    }

    public func profile(organization: Organization) throws -> Profile {
        guard let profile = try load().first(where: {
            $0.organization.name.caseInsensitiveCompare(organization.name) == .orderedSame
        }) else {
            throw ProfileStoreError.organizationNotConfigured(organization.name)
        }
        return profile
    }

    static func validateName(_ name: String) throws {
        guard !name.isEmpty, name.utf8.count <= 64 else {
            throw ProfileStoreError.invalidName(name)
        }
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-")
        guard name.unicodeScalars.allSatisfy(allowed.contains),
              let first = name.unicodeScalars.first,
              CharacterSet.alphanumerics.contains(first) else {
            throw ProfileStoreError.invalidName(name)
        }
    }

    private static func validate(_ profiles: [Profile]) throws {
        var names = Set<String>()
        var organizations = Set<String>()
        for profile in profiles {
            try validateName(profile.name)
            guard let validatedOrganization = try? Organization(profile.organization.name),
                  validatedOrganization == profile.organization,
                  !profile.identity.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  names.insert(profile.name).inserted,
                  organizations.insert(profile.organization.name.lowercased()).inserted else {
                throw ProfileStoreError.invalidProfileFile
            }
        }
    }

    private static func prepareDirectory(_ directory: URL) throws {
        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
        } catch {
            throw ProfileStoreError.io("create")
        }

        var info = stat()
        let status = directory.path.withCString { lstat($0, &info) }
        guard status == 0,
              (info.st_mode & S_IFMT) == S_IFDIR,
              info.st_uid == getuid() else {
            throw ProfileStoreError.invalidDirectory(directory.path)
        }
        guard chmod(directory.path, 0o700) == 0 else {
            throw ProfileStoreError.io("secure")
        }
    }

    private func write(_ profiles: [Profile]) throws {
        let data: Data
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            data = try encoder.encode(profiles)
        } catch {
            throw ProfileStoreError.io("encode")
        }

        let temporaryURL = directory.appendingPathComponent(".profiles-\(UUID().uuidString).tmp")
        let descriptor = temporaryURL.path.withCString {
            open($0, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        }
        guard descriptor >= 0 else {
            throw ProfileStoreError.io("write")
        }

        var shouldRemoveTemporaryFile = true
        defer {
            close(descriptor)
            if shouldRemoveTemporaryFile {
                temporaryURL.path.withCString { _ = unlink($0) }
            }
        }

        let didWrite = data.withUnsafeBytes { rawBuffer -> Bool in
            guard var pointer = rawBuffer.baseAddress else { return true }
            var remaining = rawBuffer.count
            while remaining > 0 {
                let count = Darwin.write(descriptor, pointer, remaining)
                if count < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                pointer = pointer.advanced(by: count)
                remaining -= count
            }
            return true
        }
        guard didWrite, fsync(descriptor) == 0, fchmod(descriptor, 0o600) == 0 else {
            throw ProfileStoreError.io("write")
        }

        let renameStatus = temporaryURL.path.withCString { source in
            profilesURL.path.withCString { destination in
                rename(source, destination)
            }
        }
        guard renameStatus == 0 else {
            throw ProfileStoreError.io("replace")
        }
        shouldRemoveTemporaryFile = false
    }
}
