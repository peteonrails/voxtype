import Foundation

/// Helper to run voxtype CLI commands
enum VoxtypeCLI {
    /// Path to voxtype binary
    static var binaryPath: String {
        // First try the app bundle location (works for both VoxtypeMenubar.app and VoxtypeSetup.app)
        let bundlePath = Bundle.main.bundlePath
        let parentDir = (bundlePath as NSString).deletingLastPathComponent
        let siblingBinaryPath = (parentDir as NSString).appendingPathComponent("Voxtype.app/Contents/MacOS/voxtype")

        if FileManager.default.fileExists(atPath: siblingBinaryPath) {
            return siblingBinaryPath
        }

        // Try /Applications
        let applicationsPath = "/Applications/Voxtype.app/Contents/MacOS/voxtype"
        if FileManager.default.fileExists(atPath: applicationsPath) {
            return applicationsPath
        }

        // Try homebrew symlink
        let homebrewPath = "/opt/homebrew/bin/voxtype"
        if FileManager.default.fileExists(atPath: homebrewPath) {
            return homebrewPath
        }

        // Try ~/.local/bin
        let localBinPath = NSHomeDirectory() + "/.local/bin/voxtype"
        if FileManager.default.fileExists(atPath: localBinPath) {
            return localBinPath
        }

        // Fallback to PATH
        return "voxtype"
    }

    /// Run a voxtype command
    @discardableResult
    static func run(_ arguments: [String], wait: Bool = true) -> (output: String, success: Bool) {
        let task = Process()
        task.launchPath = binaryPath
        task.arguments = arguments

        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = pipe

        do {
            try task.run()

            if wait {
                task.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                let output = String(data: data, encoding: .utf8) ?? ""
                return (output, task.terminationStatus == 0)
            } else {
                return ("", true)
            }
        } catch {
            return ("Error: \(error.localizedDescription)", false)
        }
    }

    /// Get daemon status
    static func getStatus() -> String {
        let result = run(["status"])
        return result.output.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Check if daemon is running
    static func isDaemonRunning() -> Bool {
        let result = run(["status"])
        let status = result.output.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return status == "idle" || status == "recording" || status == "transcribing"
    }
}
