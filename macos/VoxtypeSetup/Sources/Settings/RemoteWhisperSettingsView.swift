import SwiftUI

struct RemoteWhisperSettingsView: View {
    @State private var endpoint: String = ""
    @State private var apiKey: String = ""
    @State private var remoteModel: String = "whisper-1"
    @State private var timeoutSecs: Int = 30

    var body: some View {
        Form {
            Section {
                TextField("Server URL", text: $endpoint)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit {
                        saveEndpoint()
                    }

                Text("Examples:\n• whisper.cpp server: http://192.168.1.100:8080\n• OpenAI API: https://api.openai.com")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Button("Save Endpoint") {
                    saveEndpoint()
                }
            } header: {
                Text("Remote Endpoint")
            }

            Section {
                SecureField("API Key", text: $apiKey)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit {
                        saveApiKey()
                    }

                Text("Required for OpenAI API. Can also be set via VOXTYPE_WHISPER_API_KEY environment variable.")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Button("Save API Key") {
                    saveApiKey()
                }
            } header: {
                Text("Authentication")
            }

            Section {
                TextField("Model Name", text: $remoteModel)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit {
                        saveRemoteModel()
                    }

                Text("Model name to send to the remote server. Default: \"whisper-1\" for OpenAI.")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Button("Save Model") {
                    saveRemoteModel()
                }
            } header: {
                Text("Remote Model")
            }

            Section {
                Stepper("Timeout: \(timeoutSecs) seconds", value: $timeoutSecs, in: 10...120, step: 10)
                    .onChange(of: timeoutSecs) { newValue in
                        ConfigManager.shared.updateConfig(key: "remote_timeout_secs", value: "\(newValue)", section: "[whisper]")
                    }

                Text("Maximum time to wait for remote server response.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Timeout")
            }

            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text("To use remote Whisper:")
                        .fontWeight(.medium)

                    Text("1. Set Whisper mode to \"Remote\" in Whisper Settings")
                    Text("2. Enter your server URL above")
                    Text("3. Add API key if required")
                    Text("4. Restart the daemon")
                }
                .font(.caption)
                .foregroundColor(.secondary)
            } header: {
                Text("Setup Instructions")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
        }
    }

    private func loadSettings() {
        let config = ConfigManager.shared.readConfig()

        if let ep = config["whisper.remote_endpoint"]?.replacingOccurrences(of: "\"", with: "") {
            endpoint = ep
        }

        if let key = config["whisper.remote_api_key"]?.replacingOccurrences(of: "\"", with: "") {
            apiKey = key
        }

        if let model = config["whisper.remote_model"]?.replacingOccurrences(of: "\"", with: "") {
            remoteModel = model
        }

        if let timeout = config["whisper.remote_timeout_secs"], let t = Int(timeout) {
            timeoutSecs = t
        }
    }

    private func saveEndpoint() {
        if endpoint.isEmpty {
            ConfigManager.shared.updateConfig(key: "remote_endpoint", value: "# not set", section: "[whisper]")
        } else {
            ConfigManager.shared.updateConfig(key: "remote_endpoint", value: "\"\(endpoint)\"", section: "[whisper]")
        }
    }

    private func saveApiKey() {
        if apiKey.isEmpty {
            ConfigManager.shared.updateConfig(key: "remote_api_key", value: "# not set", section: "[whisper]")
        } else {
            ConfigManager.shared.updateConfig(key: "remote_api_key", value: "\"\(apiKey)\"", section: "[whisper]")
        }
    }

    private func saveRemoteModel() {
        ConfigManager.shared.updateConfig(key: "remote_model", value: "\"\(remoteModel)\"", section: "[whisper]")
    }
}
