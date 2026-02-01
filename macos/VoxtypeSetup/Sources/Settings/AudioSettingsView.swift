import SwiftUI
import AVFoundation

struct AudioSettingsView: View {
    @State private var audioDevice: String = "default"
    @State private var maxDurationSecs: Int = 60
    @State private var feedbackEnabled: Bool = false
    @State private var feedbackVolume: Double = 0.7
    @State private var availableDevices: [AudioDeviceInfo] = []

    var body: some View {
        Form {
            Section {
                Picker("Input Device", selection: $audioDevice) {
                    Text("System Default").tag("default")
                    ForEach(availableDevices, id: \.id) { device in
                        Text(device.name).tag(device.id)
                    }
                }
                .onChange(of: audioDevice) { newValue in
                    ConfigManager.shared.updateConfig(key: "device", value: "\"\(newValue)\"", section: "[audio]")
                }

                Button("Refresh Devices") {
                    loadAudioDevices()
                }

                Text("Select the microphone to use for recording.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Audio Input")
            }

            Section {
                Stepper("Maximum Recording: \(maxDurationSecs) seconds", value: $maxDurationSecs, in: 10...300, step: 10)
                    .onChange(of: maxDurationSecs) { newValue in
                        ConfigManager.shared.updateConfig(key: "max_duration_secs", value: "\(newValue)", section: "[audio]")
                    }

                Text("Safety limit to prevent accidentally long recordings.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Recording Duration")
            }

            Section {
                Toggle("Enable Audio Feedback", isOn: $feedbackEnabled)
                    .onChange(of: feedbackEnabled) { newValue in
                        ConfigManager.shared.updateConfig(key: "enabled", value: newValue ? "true" : "false", section: "[audio.feedback]")
                    }

                if feedbackEnabled {
                    HStack {
                        Text("Volume")
                        Slider(value: $feedbackVolume, in: 0...1, step: 0.1)
                            .onChange(of: feedbackVolume) { newValue in
                                ConfigManager.shared.updateConfig(key: "volume", value: String(format: "%.1f", newValue), section: "[audio.feedback]")
                            }
                        Text(String(format: "%.0f%%", feedbackVolume * 100))
                            .frame(width: 50)
                    }
                }

                Text("Play audio cues when recording starts and stops.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Audio Feedback")
            }

            Section {
                Button("Test Microphone") {
                    testMicrophone()
                }

                Text("Opens System Preferences to test your microphone.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Testing")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
            loadAudioDevices()
        }
    }

    private func loadSettings() {
        let config = ConfigManager.shared.readConfig()

        if let device = config["audio.device"]?.replacingOccurrences(of: "\"", with: "") {
            audioDevice = device
        }

        if let duration = config["audio.max_duration_secs"], let d = Int(duration) {
            maxDurationSecs = d
        }

        if let feedback = config["audio.feedback.enabled"] {
            feedbackEnabled = feedback == "true"
        }

        if let volume = config["audio.feedback.volume"], let v = Double(volume) {
            feedbackVolume = v
        }
    }

    private func loadAudioDevices() {
        availableDevices = []

        // Get audio input devices using AVFoundation
        // Use the older API for macOS 13 compatibility
        let devices = AVCaptureDevice.devices(for: .audio)

        for device in devices {
            availableDevices.append(AudioDeviceInfo(
                id: device.uniqueID,
                name: device.localizedName
            ))
        }
    }

    private func testMicrophone() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.sound?input")!
        NSWorkspace.shared.open(url)
    }
}

struct AudioDeviceInfo: Identifiable {
    let id: String
    let name: String
}
