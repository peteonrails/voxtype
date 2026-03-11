import SwiftUI

struct NotificationSettingsView: View {
    @State private var onRecordingStart: Bool = false
    @State private var onRecordingStop: Bool = false
    @State private var onTranscription: Bool = true
    @State private var showEngineIcon: Bool = false

    var body: some View {
        Form {
            Section {
                Toggle("Notify when recording starts", isOn: $onRecordingStart)
                    .onChange(of: onRecordingStart) { newValue in
                        ConfigManager.shared.updateConfig(key: "on_recording_start", value: newValue ? "true" : "false", section: "[output.notification]")
                    }

                Toggle("Notify when recording stops", isOn: $onRecordingStop)
                    .onChange(of: onRecordingStop) { newValue in
                        ConfigManager.shared.updateConfig(key: "on_recording_stop", value: newValue ? "true" : "false", section: "[output.notification]")
                    }

                Toggle("Show transcribed text", isOn: $onTranscription)
                    .onChange(of: onTranscription) { newValue in
                        ConfigManager.shared.updateConfig(key: "on_transcription", value: newValue ? "true" : "false", section: "[output.notification]")
                    }

                Text("Choose which events trigger desktop notifications.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Notification Events")
            }

            Section {
                Toggle("Show engine icon in notification", isOn: $showEngineIcon)
                    .onChange(of: showEngineIcon) { newValue in
                        ConfigManager.shared.updateConfig(key: "show_engine_icon", value: newValue ? "true" : "false", section: "[output.notification]")
                    }

                HStack(spacing: 20) {
                    VStack {
                        Text("🦜")
                            .font(.largeTitle)
                        Text("Parakeet")
                            .font(.caption)
                    }
                    VStack {
                        Text("🗣️")
                            .font(.largeTitle)
                        Text("Whisper")
                            .font(.caption)
                    }
                }
                .padding(.vertical, 8)

                Text("When enabled, notifications will include an icon indicating which transcription engine was used.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Engine Icon")
            }

            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text("macOS Notification Settings")
                        .fontWeight(.medium)

                    Text("To customize notification style, banners, and sounds:")
                        .font(.caption)
                        .foregroundColor(.secondary)

                    Button("Open System Notification Settings") {
                        openNotificationSettings()
                    }
                }
            } header: {
                Text("System Settings")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
        }
    }

    private func loadSettings() {
        let config = ConfigManager.shared.readConfig()

        if let start = config["output.notification.on_recording_start"] {
            onRecordingStart = start == "true"
        }

        if let stop = config["output.notification.on_recording_stop"] {
            onRecordingStop = stop == "true"
        }

        if let trans = config["output.notification.on_transcription"] {
            onTranscription = trans == "true"
        } else {
            // Default is true
            onTranscription = true
        }

        if let icon = config["output.notification.show_engine_icon"] {
            showEngineIcon = icon == "true"
        }
    }

    private func openNotificationSettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.notifications")!
        NSWorkspace.shared.open(url)
    }
}
