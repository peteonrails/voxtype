import SwiftUI

struct MenuBarView: View {
    @EnvironmentObject var statusMonitor: VoxtypeStatusMonitor

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Status header - single line with icon, name, and status
            Label {
                Text("Voxtype · \(statusMonitor.statusText)")
            } icon: {
                Image(systemName: statusMonitor.iconName)
                    .foregroundColor(statusColor)
            }
            .font(.headline)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            Divider()

            // Recording controls
            Button(action: toggleRecording) {
                Label("Toggle Recording", systemImage: "record.circle")
            }
            .keyboardShortcut("r", modifiers: [])
            .disabled(!statusMonitor.daemonRunning)

            Button(action: cancelRecording) {
                Label("Cancel Recording", systemImage: "xmark.circle")
            }
            .disabled(statusMonitor.state != .recording)

            Divider()

            // Quick settings menus (at top level)
            Menu("Engine") {
                Button("Parakeet (Fast)") {
                    setEngine("parakeet")
                }
                Button("Whisper") {
                    setEngine("whisper")
                }
            }

            Menu("Output Mode") {
                Button("Type Text") {
                    setOutputMode("type")
                }
                Button("Clipboard") {
                    setOutputMode("clipboard")
                }
                Button("Clipboard + Paste") {
                    setOutputMode("paste")
                }
            }

            Menu("Hotkey Mode") {
                Button("Push-to-Talk (hold)") {
                    setHotkeyMode("push_to_talk")
                }
                Button("Toggle (press)") {
                    setHotkeyMode("toggle")
                }
            }

            Divider()

            Button(action: openSettings) {
                Label("Settings", systemImage: "gearshape")
            }

            Button(action: restartDaemon) {
                Label("Restart Daemon", systemImage: "arrow.clockwise")
            }

            Button(action: viewLogs) {
                Label("View Logs", systemImage: "doc.text")
            }

            Divider()

            Button(action: quitApp) {
                Label("Quit Voxtype Menu Bar", systemImage: "power")
            }
            .keyboardShortcut("q", modifiers: .command)
        }
    }

    private var statusColor: Color {
        switch statusMonitor.state {
        case .idle:
            return .green
        case .recording:
            return .red
        case .transcribing:
            return .orange
        case .stopped:
            return .gray
        }
    }

    // MARK: - Actions

    private func toggleRecording() {
        VoxtypeCLI.run(["record", "toggle"])
    }

    private func cancelRecording() {
        VoxtypeCLI.run(["record", "cancel"])
    }

    private func setEngine(_ engine: String) {
        // Update config file
        updateConfig(key: "engine", value: "\"\(engine)\"", section: nil)
        showNotification(title: "Voxtype", message: "Engine set to \(engine). Restart daemon to apply.")
    }

    private func setOutputMode(_ mode: String) {
        updateConfig(key: "mode", value: "\"\(mode)\"", section: "[output]")
    }

    private func setHotkeyMode(_ mode: String) {
        updateConfig(key: "mode", value: "\"\(mode)\"", section: "[hotkey]")
        showNotification(title: "Voxtype", message: "Hotkey mode changed. Restart daemon to apply.")
    }

    private func openSettings() {
        // Try multiple locations for VoxtypeSetup
        let possiblePaths = [
            // Inside main app bundle
            "/Applications/Voxtype.app/Contents/MacOS/VoxtypeSetup",
            // Standalone app in Applications
            "/Applications/VoxtypeSetup.app",
            // Next to this menubar app
            Bundle.main.bundlePath.replacingOccurrences(of: "VoxtypeMenubar.app", with: "VoxtypeSetup.app"),
        ]

        for path in possiblePaths {
            if path.hasSuffix(".app") {
                // It's an app bundle
                if FileManager.default.fileExists(atPath: path) {
                    NSWorkspace.shared.open(URL(fileURLWithPath: path))
                    return
                }
            } else {
                // It's a binary
                if FileManager.default.fileExists(atPath: path) {
                    do {
                        try Process.run(URL(fileURLWithPath: path), arguments: [])
                        return
                    } catch {
                        continue
                    }
                }
            }
        }

        // Fallback: show notification that settings app not found
        showNotification(title: "Voxtype", message: "Settings app not found. Edit config at ~/Library/Application Support/voxtype/config.toml")
    }

    private func restartDaemon() {
        VoxtypeCLI.run(["daemon", "restart"], wait: false)
        showNotification(title: "Voxtype", message: "Restarting daemon...")
    }

    private func viewLogs() {
        let logsPath = NSHomeDirectory() + "/Library/Logs/voxtype"
        NSWorkspace.shared.open(URL(fileURLWithPath: logsPath))
    }

    private func quitApp() {
        NSApplication.shared.terminate(nil)
    }

    // MARK: - Helpers

    private func updateConfig(key: String, value: String, section: String?) {
        let configPath = NSHomeDirectory() + "/Library/Application Support/voxtype/config.toml"

        guard var content = try? String(contentsOfFile: configPath, encoding: .utf8) else {
            return
        }

        let pattern = "\(key)\\s*=\\s*\"[^\"]*\""
        let replacement = "\(key) = \(value)"

        if let regex = try? NSRegularExpression(pattern: pattern, options: []) {
            let range = NSRange(content.startIndex..., in: content)
            content = regex.stringByReplacingMatches(in: content, options: [], range: range, withTemplate: replacement)
        }

        try? content.write(toFile: configPath, atomically: true, encoding: .utf8)
    }

    private func showNotification(title: String, message: String) {
        let script = "display notification \"\(message)\" with title \"\(title)\""
        if let appleScript = NSAppleScript(source: script) {
            var error: NSDictionary?
            appleScript.executeAndReturnError(&error)
        }
    }
}
