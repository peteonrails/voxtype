import SwiftUI

struct WhisperSettingsView: View {
    @State private var backend: String = "local"
    @State private var language: String = "en"
    @State private var translate: Bool = false
    @State private var gpuIsolation: Bool = false
    @State private var onDemandLoading: Bool = false
    @State private var initialPrompt: String = ""

    // Remote settings
    @State private var endpoint: String = ""
    @State private var apiKey: String = ""
    @State private var remoteModel: String = "whisper-1"
    @State private var timeoutSecs: Int = 30

    private let languages = [
        ("English", "en"),
        ("Auto-detect", "auto"),
        ("Spanish", "es"),
        ("French", "fr"),
        ("German", "de"),
        ("Italian", "it"),
        ("Portuguese", "pt"),
        ("Dutch", "nl"),
        ("Polish", "pl"),
        ("Russian", "ru"),
        ("Japanese", "ja"),
        ("Chinese", "zh"),
        ("Korean", "ko"),
    ]

    var body: some View {
        Form {
            Section {
                Picker("Backend", selection: $backend) {
                    Text("Local (whisper.cpp)").tag("local")
                    Text("Remote Server").tag("remote")
                }
                .onChange(of: backend) { newValue in
                    ConfigManager.shared.updateConfig(key: "backend", value: "\"\(newValue)\"", section: "[whisper]")
                }

                Text(backend == "local"
                    ? "Run transcription locally using whisper.cpp."
                    : "Send audio to a remote Whisper server or OpenAI API.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Whisper Backend")
            }

            // Remote-only settings
            if backend == "remote" {
                Group {
                    Section {
                        TextField("Server URL", text: $endpoint)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit { saveEndpoint() }

                        Text("Examples: http://192.168.1.100:8080 or https://api.openai.com")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    } header: {
                        Text("Remote Endpoint")
                    }

                    Section {
                        SecureField("API Key", text: $apiKey)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit { saveApiKey() }

                        Text("Required for OpenAI API. Can also use VOXTYPE_WHISPER_API_KEY env var.")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    } header: {
                        Text("Authentication")
                    }

                    Section {
                        TextField("Model Name", text: $remoteModel)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit { saveRemoteModel() }

                        Stepper("Timeout: \(timeoutSecs)s", value: $timeoutSecs, in: 10...120, step: 10)
                            .onChange(of: timeoutSecs) { newValue in
                                ConfigManager.shared.updateConfig(key: "remote_timeout_secs", value: "\(newValue)", section: "[whisper]")
                            }
                    } header: {
                        Text("Remote Options")
                    }
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            // Local-only settings
            if backend == "local" {
                Section {
                    Toggle("GPU Isolation", isOn: $gpuIsolation)
                        .onChange(of: gpuIsolation) { newValue in
                            ConfigManager.shared.updateConfig(key: "gpu_isolation", value: newValue ? "true" : "false", section: "[whisper]")
                        }

                    Text("Run in subprocess that exits after use, releasing GPU memory.")
                        .font(.caption)
                        .foregroundColor(.secondary)

                    Toggle("On-Demand Loading", isOn: $onDemandLoading)
                        .onChange(of: onDemandLoading) { newValue in
                            ConfigManager.shared.updateConfig(key: "on_demand_loading", value: newValue ? "true" : "false", section: "[whisper]")
                        }

                    Text("Load model only when recording. Saves memory but adds latency.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                } header: {
                    Text("Performance")
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            // Shared settings (both local and remote)
            Section {
                Picker("Language", selection: $language) {
                    ForEach(languages, id: \.1) { name, code in
                        Text(name).tag(code)
                    }
                }
                .onChange(of: language) { newValue in
                    ConfigManager.shared.updateConfig(key: "language", value: "\"\(newValue)\"", section: "[whisper]")
                }

                Toggle("Translate to English", isOn: $translate)
                    .onChange(of: translate) { newValue in
                        ConfigManager.shared.updateConfig(key: "translate", value: newValue ? "true" : "false", section: "[whisper]")
                    }

                Text("Translate non-English speech to English.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Language")
            }

            Section {
                TextField("Initial Prompt", text: $initialPrompt, axis: .vertical)
                    .lineLimit(2...4)
                    .onSubmit { saveInitialPrompt() }

                Text("Hint at terminology or formatting. Example: \"Technical discussion about Rust.\"")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Initial Prompt")
            }
        }
        .formStyle(.grouped)
        .animation(.easeInOut(duration: 0.25), value: backend)
        .onAppear {
            loadSettings()
        }
    }

    private func loadSettings() {
        let config = ConfigManager.shared.readConfig()

        if let b = config["whisper.backend"]?.replacingOccurrences(of: "\"", with: "") {
            backend = b
        }

        if let lang = config["whisper.language"]?.replacingOccurrences(of: "\"", with: "") {
            language = lang
        }

        if let trans = config["whisper.translate"] {
            translate = trans == "true"
        }

        if let gpu = config["whisper.gpu_isolation"] {
            gpuIsolation = gpu == "true"
        }

        if let onDemand = config["whisper.on_demand_loading"] {
            onDemandLoading = onDemand == "true"
        }

        if let prompt = config["whisper.initial_prompt"]?.replacingOccurrences(of: "\"", with: "") {
            initialPrompt = prompt
        }

        // Remote settings
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

    private func saveInitialPrompt() {
        if initialPrompt.isEmpty {
            ConfigManager.shared.updateConfig(key: "initial_prompt", value: "\"\"", section: "[whisper]")
        } else {
            let escaped = initialPrompt.replacingOccurrences(of: "\"", with: "\\\"")
            ConfigManager.shared.updateConfig(key: "initial_prompt", value: "\"\(escaped)\"", section: "[whisper]")
        }
    }

    private func saveEndpoint() {
        if endpoint.isEmpty {
            ConfigManager.shared.updateConfig(key: "remote_endpoint", value: "\"\"", section: "[whisper]")
        } else {
            ConfigManager.shared.updateConfig(key: "remote_endpoint", value: "\"\(endpoint)\"", section: "[whisper]")
        }
    }

    private func saveApiKey() {
        if apiKey.isEmpty {
            ConfigManager.shared.updateConfig(key: "remote_api_key", value: "\"\"", section: "[whisper]")
        } else {
            ConfigManager.shared.updateConfig(key: "remote_api_key", value: "\"\(apiKey)\"", section: "[whisper]")
        }
    }

    private func saveRemoteModel() {
        ConfigManager.shared.updateConfig(key: "remote_model", value: "\"\(remoteModel)\"", section: "[whisper]")
    }
}
