import SwiftUI

struct OutputSettingsView: View {
    @State private var outputMode: String = "type"
    @State private var fallbackToClipboard: Bool = true
    @State private var typeDelayMs: Int = 0
    @State private var autoSubmit: Bool = false

    var body: some View {
        Form {
            Section {
                Picker("Output Mode", selection: $outputMode) {
                    Text("Type Text").tag("type")
                    Text("Copy to Clipboard").tag("clipboard")
                    Text("Clipboard + Paste").tag("paste")
                }
                .onChange(of: outputMode) { newValue in
                    updateConfig(key: "mode", value: "\"\(newValue)\"", section: "[output]")
                }

                Text(outputModeDescription)
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Output Mode")
            }

            Section {
                Toggle("Fall back to clipboard if typing fails", isOn: $fallbackToClipboard)
                    .onChange(of: fallbackToClipboard) { newValue in
                        updateConfig(key: "fallback_to_clipboard", value: newValue ? "true" : "false", section: "[output]")
                    }

                Stepper("Type delay: \(typeDelayMs) ms", value: $typeDelayMs, in: 0...100, step: 5)
                    .onChange(of: typeDelayMs) { newValue in
                        updateConfig(key: "type_delay_ms", value: "\(newValue)", section: "[output]")
                    }

                Text("Increase delay if characters are being dropped.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Typing Options")
            }

            Section {
                Toggle("Auto-submit after transcription", isOn: $autoSubmit)
                    .onChange(of: autoSubmit) { newValue in
                        updateConfig(key: "auto_submit", value: newValue ? "true" : "false", section: "[output]")
                    }

                Text("Press Enter automatically after typing transcribed text.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Behavior")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
        }
    }

    private var outputModeDescription: String {
        switch outputMode {
        case "type":
            return "Text is typed directly into the active application."
        case "clipboard":
            return "Text is copied to clipboard. Paste manually with ⌘V."
        case "paste":
            return "Text is copied to clipboard and pasted automatically."
        default:
            return ""
        }
    }

    private func loadSettings() {
        let config = readConfig()

        if let mode = config["output.mode"]?.replacingOccurrences(of: "\"", with: "") {
            outputMode = mode
        }

        if let fallback = config["output.fallback_to_clipboard"] {
            fallbackToClipboard = fallback == "true"
        }

        if let delay = config["output.type_delay_ms"], let value = Int(delay) {
            typeDelayMs = value
        }

        if let submit = config["output.auto_submit"] {
            autoSubmit = submit == "true"
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

        let pattern = "\(key)\\s*=\\s*[^\\n]*"
        let replacement = "\(key) = \(value)"

        if let regex = try? NSRegularExpression(pattern: pattern, options: []) {
            let range = NSRange(content.startIndex..., in: content)
            content = regex.stringByReplacingMatches(in: content, options: [], range: range, withTemplate: replacement)
        }

        try? content.write(toFile: configPath, atomically: true, encoding: .utf8)
    }
}
