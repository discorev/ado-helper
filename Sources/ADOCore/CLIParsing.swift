import Foundation

public enum CLIParseError: LocalizedError, Equatable {
    case message(String)

    public var errorDescription: String? {
        switch self {
        case .message(let message): return message
        }
    }
}

public enum PullRequestTarget: Equatable {
    case currentBranch
    case number(Int)
    case url(String)
}

public enum CLICommand: Equatable {
    case help
    case authAdd(name: String, organization: String, openBrowser: Bool)
    case authUpdate(name: String, openBrowser: Bool)
    case authStatus(check: Bool)
    case authRemove(name: String)
    case prShow(target: PullRequestTarget, profile: String?)
    case prThreads(target: PullRequestTarget, profile: String?)
    case prChanges(target: PullRequestTarget, profile: String?, iteration: Int?)
    case prClone(target: PullRequestTarget, profile: String?, directory: String?)
    case prDiff(target: PullRequestTarget, profile: String?, directory: String?)
    case prComment(target: PullRequestTarget, profile: String?, file: String, line: Int,
                   endLine: Int, side: String, bodyFile: String, commit: String,
                   iteration: Int, changeID: Int)
}

public struct CLIParser {
    public init() {}

    public func parse(_ arguments: [String]) throws -> CLICommand {
        guard let group = arguments.first else { return .help }
        if ["help", "--help", "-h"].contains(group) { return .help }
        guard arguments.count >= 2 else {
            throw CLIParseError.message("Missing command. Run 'ado help' for usage.")
        }
        switch (group, arguments[1]) {
        case ("auth", "add"): return try parseAuthAdd(Array(arguments.dropFirst(2)))
        case ("auth", "update"): return try parseAuthUpdate(Array(arguments.dropFirst(2)))
        case ("auth", "status"): return try parseAuthStatus(Array(arguments.dropFirst(2)))
        case ("auth", "remove"): return try parseAuthRemove(Array(arguments.dropFirst(2)))
        case ("pr", "show"): return try parseSimplePR(.show, Array(arguments.dropFirst(2)))
        case ("pr", "threads"): return try parseSimplePR(.threads, Array(arguments.dropFirst(2)))
        case ("pr", "changes"): return try parseChanges(Array(arguments.dropFirst(2)))
        case ("pr", "clone"): return try parseCheckout(.clone, Array(arguments.dropFirst(2)))
        case ("pr", "diff"): return try parseCheckout(.diff, Array(arguments.dropFirst(2)))
        case ("pr", "comment"): return try parseComment(Array(arguments.dropFirst(2)))
        default: throw CLIParseError.message("Unknown command '\(group) \(arguments[1])'. Run 'ado help' for usage.")
        }
    }

    private func parseAuthAdd(_ args: [String]) throws -> CLICommand {
        var cursor = ArgumentCursor(args)
        let name = try cursor.requiredPositional("profile name")
        var organization: String?
        var openBrowser = true
        var sawNoBrowser = false
        while let option = cursor.next() {
            switch option {
            case "--org":
                guard organization == nil else { throw CLIParseError.message("Option '--org' may only be specified once.") }
                organization = try cursor.requiredValue(after: option)
            case "--no-browser":
                guard !sawNoBrowser else { throw CLIParseError.message("Option '--no-browser' may only be specified once.") }
                sawNoBrowser = true; openBrowser = false
            default: throw cursor.unexpected(option)
            }
        }
        guard let organization else { throw CLIParseError.message("auth add requires --org URL.") }
        return .authAdd(name: name, organization: organization, openBrowser: openBrowser)
    }

    private func parseAuthUpdate(_ args: [String]) throws -> CLICommand {
        var cursor = ArgumentCursor(args)
        let name = try cursor.requiredPositional("profile name")
        var openBrowser = true
        var sawNoBrowser = false
        while let option = cursor.next() {
            switch option {
            case "--no-browser":
                guard !sawNoBrowser else { throw CLIParseError.message("Option '--no-browser' may only be specified once.") }
                sawNoBrowser = true; openBrowser = false
            default: throw cursor.unexpected(option)
            }
        }
        return .authUpdate(name: name, openBrowser: openBrowser)
    }

    private func parseAuthStatus(_ args: [String]) throws -> CLICommand {
        guard args.allSatisfy({ $0 == "--check" }), args.filter({ $0 == "--check" }).count <= 1 else {
            throw CLIParseError.message("Usage: ado auth status [--check]")
        }
        return .authStatus(check: args.contains("--check"))
    }

    private func parseAuthRemove(_ args: [String]) throws -> CLICommand {
        guard args.count == 1, !args[0].hasPrefix("-") else {
            throw CLIParseError.message("Usage: ado auth remove NAME")
        }
        return .authRemove(name: args[0])
    }

    private enum SimplePRCommand { case show, threads }
    private func parseSimplePR(_ command: SimplePRCommand, _ args: [String]) throws -> CLICommand {
        var values = try parseCommonPR(args, allowed: ["--profile"])
        let target = try values.target()
        let profile = values.take("--profile")
        try values.ensureEmpty()
        return command == .show ? .prShow(target: target, profile: profile) : .prThreads(target: target, profile: profile)
    }

    private func parseChanges(_ args: [String]) throws -> CLICommand {
        var values = try parseCommonPR(args, allowed: ["--profile", "--iteration"])
        let target = try values.target()
        let profile = values.take("--profile")
        let iteration = try values.positiveInt("--iteration")
        try values.ensureEmpty()
        return .prChanges(target: target, profile: profile, iteration: iteration)
    }

    private enum CheckoutCommand { case clone, diff }
    private func parseCheckout(_ command: CheckoutCommand, _ args: [String]) throws -> CLICommand {
        var values = try parseCommonPR(args, allowed: ["--profile", "--directory"])
        let target = try values.target()
        let profile = values.take("--profile")
        let directory = values.take("--directory")
        try values.ensureEmpty()
        return command == .clone
            ? .prClone(target: target, profile: profile, directory: directory)
            : .prDiff(target: target, profile: profile, directory: directory)
    }

    private func parseComment(_ args: [String]) throws -> CLICommand {
        let names = ["--profile", "--file", "--line", "--end-line", "--side", "--body-file", "--commit", "--iteration", "--change-id"]
        var values = try parseCommonPR(args, allowed: Set(names))
        let target = try values.target()
        let profile = values.take("--profile")
        let file = try values.required("--file")
        let line = try values.requiredPositiveInt("--line")
        let endLine = try values.positiveInt("--end-line") ?? line
        guard endLine >= line else { throw CLIParseError.message("--end-line must be greater than or equal to --line.") }
        let side = try values.required("--side")
        guard side == "left" || side == "right" else { throw CLIParseError.message("--side must be 'left' or 'right'.") }
        let bodyFile = try values.required("--body-file")
        let commit = try values.required("--commit")
        guard GitValidation.isCommitSHA(commit) else { throw CLIParseError.message("--commit must be a 40-character hexadecimal commit SHA.") }
        let iteration = try values.requiredPositiveInt("--iteration")
        let changeID = try values.requiredPositiveInt("--change-id")
        try values.ensureEmpty()
        return .prComment(target: target, profile: profile, file: file, line: line,
                          endLine: endLine, side: side, bodyFile: bodyFile, commit: commit,
                          iteration: iteration, changeID: changeID)
    }

    private func parseCommonPR(_ args: [String], allowed: Set<String>) throws -> ParsedOptions {
        var result = ParsedOptions()
        var index = 0
        while index < args.count {
            let value = args[index]
            if value.hasPrefix("-") {
                guard allowed.contains(value) else { throw CLIParseError.message("Unexpected option '\(value)'.") }
                guard result.options[value] == nil else { throw CLIParseError.message("Option '\(value)' may only be specified once.") }
                guard index + 1 < args.count else { throw CLIParseError.message("Option '\(value)' requires a value.") }
                let next = args[index + 1]
                guard !next.hasPrefix("--") else { throw CLIParseError.message("Option '\(value)' requires a value.") }
                result.options[value] = next
                index += 2
            } else {
                guard result.positional == nil else { throw CLIParseError.message("Only one pull request target may be specified.") }
                result.positional = value
                index += 1
            }
        }
        return result
    }
}

private struct ArgumentCursor {
    let arguments: [String]
    var index = 0
    init(_ arguments: [String]) { self.arguments = arguments }
    mutating func next() -> String? {
        guard index < arguments.count else { return nil }
        defer { index += 1 }
        return arguments[index]
    }
    mutating func requiredPositional(_ label: String) throws -> String {
        guard let value = next(), !value.hasPrefix("-") else { throw CLIParseError.message("Missing \(label).") }
        return value
    }
    mutating func requiredValue(after option: String) throws -> String {
        guard let value = next(), !value.hasPrefix("--") else { throw CLIParseError.message("Option '\(option)' requires a value.") }
        return value
    }
    func unexpected(_ value: String) -> CLIParseError { .message("Unexpected argument '\(value)'.") }
}

private struct ParsedOptions {
    var positional: String?
    var options: [String: String] = [:]
    mutating func target() throws -> PullRequestTarget {
        guard let raw = positional else { return .currentBranch }
        positional = nil
        if raw.lowercased().hasPrefix("https://") { return .url(raw) }
        guard let id = Int(raw), id > 0 else {
            throw CLIParseError.message("TARGET must be an Azure DevOps pull request URL or a positive pull request number.")
        }
        return .number(id)
    }
    mutating func take(_ name: String) -> String? { options.removeValue(forKey: name) }
    mutating func required(_ name: String) throws -> String {
        guard let value = take(name), !value.isEmpty else { throw CLIParseError.message("pr comment requires \(name) VALUE.") }
        return value
    }
    mutating func positiveInt(_ name: String) throws -> Int? {
        guard let raw = take(name) else { return nil }
        guard let value = Int(raw), value > 0 else { throw CLIParseError.message("\(name) must be a positive integer.") }
        return value
    }
    mutating func requiredPositiveInt(_ name: String) throws -> Int {
        guard let value = try positiveInt(name) else { throw CLIParseError.message("pr comment requires \(name) N.") }
        return value
    }
    func ensureEmpty() throws {
        if let option = options.keys.sorted().first { throw CLIParseError.message("Unexpected option '\(option)'.") }
        if positional != nil { throw CLIParseError.message("Unexpected pull request target.") }
    }
}
