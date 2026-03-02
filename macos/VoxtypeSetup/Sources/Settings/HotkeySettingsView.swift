import SwiftUI

struct HotkeySettingsView: View {
    @State private var hotkeyEnabled: Bool = true
    @State private var hotkey: String = "RIGHTALT"
    @State private var hotkeyMode: String = "push_to_talk"
    @State private var cancelKey: String = ""
    @State private var modelModifier: String = ""
    @State private var modifiers: [String] = []
    @State private var needsRestart: Bool = false

    private let availableKeys = [
        ("Right Option (⌥)", "RIGHTALT"),
        ("Right Command (⌘)", "RIGHTMETA"),
        ("Right Control (⌃)", "RIGHTCTRL"),
        ("Left Option (⌥)", "LEFTALT"),
        ("Left Command (⌘)", "LEFTMETA"),
        ("Left Control (⌃)", "LEFTCTRL"),
        ("F13", "F13"),
        ("F14", "F14"),
        ("F15", "F15"),
        ("F16", "F16"),
        ("F17", "F17"),
        ("F18", "F18"),
        ("F19", "F19"),
        ("Scroll Lock", "SCROLLLOCK"),
        ("Pause", "PAUSE"),
    ]

    private let availableModifiers = [
        ("None", ""),
        ("Left Shift", "LEFTSHIFT"),
        ("Right Shift", "RIGHTSHIFT"),
        ("Left Control", "LEFTCTRL"),
        ("Right Control", "RIGHTCTRL"),
        ("Left Option", "LEFTALT"),
        ("Right Option", "RIGHTALT"),
    ]

    var body: some View {
        Form {
            if needsRestart {
                Section {
                    HStack {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundColor(.orange)
                        Text("Restart daemon to apply hotkey changes")
                        Spacer()
                        Button("Restart Now") {
                            restartDaemon()
                        }
                        .buttonStyle(.borderedProminent)
                    }
                }
            }

            Section {
                Toggle("Enable built-in hotkey detection", isOn: $hotkeyEnabled)
                    .onChange(of: hotkeyEnabled) { newValue in
                        updateConfig(key: "enabled", value: newValue ? "true" : "false", section: "[hotkey]")
                        needsRestart = true
                    }

                Text("Disable if using compositor keybindings (Hyprland, Sway) instead.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Hotkey Detection")
            }

            Section {
                Picker("Hotkey", selection: $hotkey) {
                    ForEach(availableKeys, id: \.1) { name, value in
                        Text(name).tag(value)
                    }
                }
                .onChange(of: hotkey) { newValue in
                    updateConfig(key: "key", value: "\"\(newValue)\"", section: "[hotkey]")
                    needsRestart = true
                }

                Picker("Mode", selection: $hotkeyMode) {
                    Text("Push-to-Talk (hold to record)").tag("push_to_talk")
                    Text("Toggle (press to start/stop)").tag("toggle")
                }
                .onChange(of: hotkeyMode) { newValue in
                    updateConfig(key: "mode", value: "\"\(newValue)\"", section: "[hotkey]")
                    needsRestart = true
                }

                Text(hotkeyMode == "push_to_talk"
                    ? "Hold the hotkey to record, release to transcribe."
                    : "Press once to start recording, press again to stop.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Primary Hotkey")
            }

            Section {
                Picker("Cancel Key", selection: $cancelKey) {
                    Text("None").tag("")
                    Text("Escape").tag("ESC")
                    Text("Backspace").tag("BACKSPACE")
                    Text("F12").tag("F12")
                }
                .onChange(of: cancelKey) { newValue in
                    if newValue.isEmpty {
                        // Remove the key from config
                        updateConfig(key: "cancel_key", value: "# disabled", section: "[hotkey]")
                    } else {
                        updateConfig(key: "cancel_key", value: "\"\(newValue)\"", section: "[hotkey]")
                    }
                    needsRestart = true
                }

                Text("Press this key to cancel the current recording or transcription.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Cancel Key")
            }

            Section {
                Picker("Model Modifier", selection: $modelModifier) {
                    ForEach(availableModifiers, id: \.1) { name, value in
                        Text(name).tag(value)
                    }
                }
                .onChange(of: modelModifier) { newValue in
                    if newValue.isEmpty {
                        updateConfig(key: "model_modifier", value: "# disabled", section: "[hotkey]")
                    } else {
                        updateConfig(key: "model_modifier", value: "\"\(newValue)\"", section: "[hotkey]")
                    }
                    needsRestart = true
                }

                Text("Hold this modifier with the hotkey to use a secondary model (e.g., larger model for difficult audio).")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Secondary Model Modifier")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
        }
    }

    private func loadSettings() {
        let config = ConfigManager.shared.readConfig()

        if let enabled = config["hotkey.enabled"] {
            hotkeyEnabled = enabled == "true"
        }

        if let key = config["hotkey.key"]?.replacingOccurrences(of: "\"", with: "") {
            hotkey = key
        }

        if let mode = config["hotkey.mode"]?.replacingOccurrences(of: "\"", with: "") {
            hotkeyMode = mode
        }

        if let cancel = config["hotkey.cancel_key"]?.replacingOccurrences(of: "\"", with: "") {
            cancelKey = cancel
        }

        if let modifier = config["hotkey.model_modifier"]?.replacingOccurrences(of: "\"", with: "") {
            modelModifier = modifier
        }
    }

    private func updateConfig(key: String, value: String, section: String? = nil) {
        ConfigManager.shared.updateConfig(key: key, value: value, section: section)
    }

    private func restartDaemon() {
        VoxtypeCLI.restartDaemon {
            self.needsRestart = false
        }
    }
}
