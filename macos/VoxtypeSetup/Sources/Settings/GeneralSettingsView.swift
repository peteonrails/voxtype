import SwiftUI

struct GeneralSettingsView: View {
    @State private var selectedEngine: String = "parakeet"
    @State private var hotkeyMode: String = "push_to_talk"
    @State private var hotkey: String = "RIGHTALT"
    @State private var daemonRunning: Bool = false

    var body: some View {
        Form {
            Section {
                Picker("Transcription Engine", selection: $selectedEngine) {
                    Text("Parakeet (Fast)").tag("parakeet")
                    Text("Whisper").tag("whisper")
                }
                .onChange(of: selectedEngine) { newValue in
                    updateConfig(key: "engine", value: "\"\(newValue)\"")
                }

                Text("Parakeet is faster and recommended for most users.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Engine")
            }

            Section {
                Picker("Hotkey", selection: $hotkey) {
                    Text("Right Option (⌥)").tag("RIGHTALT")
                    Text("Right Command (⌘)").tag("RIGHTMETA")
                    Text("Right Control (⌃)").tag("RIGHTCTRL")
                    Text("F13").tag("F13")
                    Text("F14").tag("F14")
                    Text("F15").tag("F15")
                }
                .onChange(of: hotkey) { newValue in
                    updateConfig(key: "key", value: "\"\(newValue)\"", section: "[hotkey]")
                }

                Picker("Mode", selection: $hotkeyMode) {
                    Text("Push-to-Talk (hold to record)").tag("push_to_talk")
                    Text("Toggle (press to start/stop)").tag("toggle")
                }
                .onChange(of: hotkeyMode) { newValue in
                    updateConfig(key: "mode", value: "\"\(newValue)\"", section: "[hotkey]")
                }
            } header: {
                Text("Hotkey")
            }

            Section {
                HStack {
                    Circle()
                        .fill(daemonRunning ? Color.green : Color.red)
                        .frame(width: 10, height: 10)
                    Text(daemonRunning ? "Daemon is running" : "Daemon is not running")

                    Spacer()

                    if daemonRunning {
                        Button("Restart") {
                            restartDaemon()
                        }
                    } else {
                        Button("Start") {
                            startDaemon()
                        }
                    }
                }
            } header: {
                Text("Daemon Status")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
            checkDaemonStatus()
        }
    }

    private func loadSettings() {
        let config = readConfig()

        if let engine = config["engine"] {
            selectedEngine = engine.replacingOccurrences(of: "\"", with: "")
        }

        if let key = config["hotkey.key"] {
            hotkey = key.replacingOccurrences(of: "\"", with: "")
        }

        if let mode = config["hotkey.mode"] {
            hotkeyMode = mode.replacingOccurrences(of: "\"", with: "")
        }
    }

    private func checkDaemonStatus() {
        let result = VoxtypeCLI.run(["status"])
        let status = result.output.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        daemonRunning = (status == "idle" || status == "recording" || status == "transcribing")
    }

    private func startDaemon() {
        VoxtypeCLI.run(["daemon"], wait: false)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            checkDaemonStatus()
        }
    }

    private func restartDaemon() {
        let task = Process()
        task.launchPath = "/bin/launchctl"
        task.arguments = ["kickstart", "-k", "gui/\(getuid())/io.voxtype.daemon"]
        try? task.run()

        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            checkDaemonStatus()
        }
    }

    private func readConfig() -> [String: String] {
        let configPath = NSHomeDirectory() + "/Library/Application Support/voxtype/config.toml"
        guard let content = try? String(contentsOfFile: configPath, encoding: .utf8) else {
            return [:]
        }

        var result: [String: String] = [:]
        var currentSection = ""

        for line in content.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                currentSection = String(trimmed.dropFirst().dropLast())
            } else if trimmed.contains("=") && !trimmed.hasPrefix("#") {
                let parts = trimmed.components(separatedBy: "=")
                if parts.count >= 2 {
                    let key = parts[0].trimmingCharacters(in: .whitespaces)
                    let value = parts.dropFirst().joined(separator: "=").trimmingCharacters(in: .whitespaces)
                    let fullKey = currentSection.isEmpty ? key : "\(currentSection).\(key)"
                    result[fullKey] = value
                }
            }
        }

        return result
    }

    private func updateConfig(key: String, value: String, section: String? = nil) {
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
}
