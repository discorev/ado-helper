import Darwin
import Foundation

public enum TerminalError: LocalizedError, Equatable {
    case noInteractiveTerminal
    case readFailed
    case secretTooLong

    public var errorDescription: String? {
        switch self {
        case .noInteractiveTerminal:
            return "Authentication needs an interactive terminal. Run this command directly in a terminal."
        case .readFailed:
            return "Could not read from the interactive terminal."
        case .secretTooLong:
            return "The entered token is unexpectedly long."
        }
    }
}

public enum Terminal {
    public static func requireInteractive() throws {
        let descriptor = open("/dev/tty", O_RDWR | O_CLOEXEC)
        guard descriptor >= 0 else {
            throw TerminalError.noInteractiveTerminal
        }
        defer { close(descriptor) }
        guard isatty(descriptor) == 1 else {
            throw TerminalError.noInteractiveTerminal
        }
    }

    public static func readSecret(prompt: String) throws -> String {
        var buffer = [CChar](repeating: 0, count: 4097)
        let result = prompt.withCString { promptPointer in
            readpassphrase(promptPointer, &buffer, buffer.count, RPP_REQUIRE_TTY)
        }
        guard result != nil else {
            if errno == ENOTTY || errno == ENXIO || errno == ENOENT {
                throw TerminalError.noInteractiveTerminal
            }
            throw TerminalError.readFailed
        }
        defer { _ = buffer.withUnsafeMutableBytes { $0.initializeMemory(as: UInt8.self, repeating: 0) } }

        guard buffer[buffer.count - 1] == 0 else {
            throw TerminalError.secretTooLong
        }
        return String(cString: buffer)
    }

    public static func confirm(prompt: String) throws -> Bool {
        let descriptor = open("/dev/tty", O_RDWR | O_CLOEXEC)
        guard descriptor >= 0, isatty(descriptor) == 1 else {
            if descriptor >= 0 { close(descriptor) }
            throw TerminalError.noInteractiveTerminal
        }
        defer { close(descriptor) }

        try writeAll("\(prompt) [y/N] ", to: descriptor)
        var bytes = [UInt8]()
        var byte: UInt8 = 0
        while bytes.count < 32 {
            let count = Darwin.read(descriptor, &byte, 1)
            if count < 0, errno == EINTR { continue }
            guard count == 1 else { throw TerminalError.readFailed }
            if byte == 10 || byte == 13 { break }
            bytes.append(byte)
        }
        guard bytes.count < 32, let response = String(bytes: bytes, encoding: .utf8) else {
            throw TerminalError.readFailed
        }
        return response.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "y"
            || response.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "yes"
    }

    private static func writeAll(_ value: String, to descriptor: Int32) throws {
        let data = Data(value.utf8)
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
        guard didWrite else { throw TerminalError.readFailed }
    }
}
