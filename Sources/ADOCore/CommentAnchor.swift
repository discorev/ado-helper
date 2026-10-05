import Foundation

public enum CommentAnchorError: LocalizedError, Equatable {
    case invalidSide
    case missingChangeType
    case sideUnavailable(side: String, changeType: String)
    case lineOutOfRange(endLine: Int, lineCount: Int)

    public var errorDescription: String? {
        switch self {
        case .invalidSide:
            return "Comment side must be left or right."
        case .missingChangeType:
            return "Azure DevOps did not return the change type needed to validate this comment anchor."
        case .sideUnavailable(let side, let changeType):
            return "The \(side) side does not exist for an Azure DevOps '\(changeType)' change."
        case .lineOutOfRange(let endLine, let lineCount):
            let unit = lineCount == 1 ? "line" : "lines"
            return "Comment line \(endLine) is outside the selected file version, which has \(lineCount) \(unit)."
        }
    }
}

public enum CommentAnchor {
    public static func validateSide(_ side: String, changeType: Any?) throws {
        guard side == "left" || side == "right" else { throw CommentAnchorError.invalidSide }
        let flags = try changeFlags(changeType)
        if side == "left", flags.addition {
            throw CommentAnchorError.sideUnavailable(side: side, changeType: flags.description)
        }
        if side == "right", flags.deletion {
            throw CommentAnchorError.sideUnavailable(side: side, changeType: flags.description)
        }
    }

    public static func validate(side: String, startLine: Int, endLine: Int,
                                content: String, changeType: Any?) throws {
        try validateSide(side, changeType: changeType)
        let count = lineCount(in: content)
        guard startLine > 0, endLine >= startLine, endLine <= count else {
            throw CommentAnchorError.lineOutOfRange(endLine: endLine, lineCount: count)
        }
    }

    /// Counts addressable text lines. A final newline terminates the preceding line and does not
    /// create an extra phantom line; an empty file has no addressable lines.
    public static func lineCount(in content: String) -> Int {
        guard !content.isEmpty else { return 0 }
        let newlines = content.utf8.reduce(into: 0) { if $1 == 0x0A { $0 += 1 } }
        return content.utf8.last == 0x0A ? newlines : newlines + 1
    }

    private static func changeFlags(_ raw: Any?) throws -> (addition: Bool, deletion: Bool, description: String) {
        if let number = raw as? NSNumber {
            let value = number.intValue
            let undelete = value & 32 != 0
            return (value & 1 != 0 || undelete, value & 16 != 0 && !undelete, String(value))
        }
        guard let value = raw as? String, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw CommentAnchorError.missingChangeType
        }
        let tokens = Set(value.lowercased().split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init))
        let undelete = tokens.contains("undelete")
        return (tokens.contains("add") || undelete, tokens.contains("delete") && !undelete, value)
    }
}
