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
        if let mode = ConfigManager.shared.getString("output.mode") {
            outputMode = mode
        }

        if let fallback = ConfigManager.shared.getBool("output.fallback_to_clipboard") {
            fallbackToClipboard = fallback
        }

        if let delay = ConfigManager.shared.getInt("output.type_delay_ms") {
            typeDelayMs = delay
        }

        if let submit = ConfigManager.shared.getBool("output.auto_submit") {
            autoSubmit = submit
        }
    }

    private func updateConfig(key: String, value: String, section: String? = nil) {
        ConfigManager.shared.updateConfig(key: key, value: value, section: section)
    }
}
