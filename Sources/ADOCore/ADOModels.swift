import Foundation

public struct ADOIdentity: Codable, Equatable {
    public let id: String
    public let displayName: String
    public let uniqueName: String

    public init(id: String, displayName: String, uniqueName: String) {
        self.id = id
        self.displayName = displayName
        self.uniqueName = uniqueName
    }
}
