import SwiftUI

struct ModelsSettingsView: View {
    @State private var installedModels: Set<String> = []
    @State private var selectedModel: String = ""
    @State private var downloadingModel: String? = nil
    @State private var needsRestart: Bool = false

    private let allModels: [ModelCategory] = [
        ModelCategory(name: "Parakeet", description: "Fast, English-only, recommended", models: [
            ModelDefinition(id: "parakeet-tdt-0.6b-v3-int8", name: "Parakeet INT8", size: "~640 MB", description: "Quantized, fastest"),
            ModelDefinition(id: "parakeet-tdt-0.6b-v3", name: "Parakeet Full", size: "~1.2 GB", description: "Full precision"),
        ]),
        ModelCategory(name: "Whisper English", description: "OpenAI Whisper, optimized for English", models: [
            ModelDefinition(id: "base.en", name: "Base English", size: "~142 MB", description: "Fast, good accuracy"),
            ModelDefinition(id: "small.en", name: "Small English", size: "~466 MB", description: "Better accuracy"),
            ModelDefinition(id: "medium.en", name: "Medium English", size: "~1.5 GB", description: "High accuracy"),
        ]),
        ModelCategory(name: "Whisper Multilingual", description: "Supports 99 languages", models: [
            ModelDefinition(id: "base", name: "Base", size: "~142 MB", description: "Fast, 99 languages"),
            ModelDefinition(id: "small", name: "Small", size: "~466 MB", description: "Better accuracy"),
            ModelDefinition(id: "medium", name: "Medium", size: "~1.5 GB", description: "High accuracy"),
            ModelDefinition(id: "large-v3", name: "Large V3", size: "~3.1 GB", description: "Best quality"),
            ModelDefinition(id: "large-v3-turbo", name: "Large V3 Turbo", size: "~1.6 GB", description: "Fast, near-large quality"),
        ]),
    ]

    var body: some View {
        Form {
            if needsRestart {
                Section {
                    HStack {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundColor(.orange)
                        Text("Model changed. Restart daemon to apply.")
                        Spacer()
                        Button("Restart Now") {
                            restartDaemon()
                            needsRestart = false
                        }
                        .buttonStyle(.borderedProminent)
                    }
                }
            }

            ForEach(allModels, id: \.name) { category in
                Section {
                    ForEach(category.models, id: \.id) { model in
                        ModelRowView(
                            model: model,
                            isInstalled: installedModels.contains(model.id),
                            isSelected: selectedModel == model.id,
                            isDownloading: downloadingModel == model.id,
                            onSelect: { selectModel(model.id) },
                            onDownload: { downloadModel(model.id) }
                        )
                    }
                } header: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(category.name)
                        Text(category.description)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .fontWeight(.regular)
                    }
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadInstalledModels()
        }
    }

    private func loadInstalledModels() {
        let modelsDir = NSHomeDirectory() + "/Library/Application Support/voxtype/models"

        guard let contents = try? FileManager.default.contentsOfDirectory(atPath: modelsDir) else {
            return
        }

        var installed: Set<String> = []

        for item in contents {
            let path = modelsDir + "/" + item

            var isDir: ObjCBool = false
            FileManager.default.fileExists(atPath: path, isDirectory: &isDir)

            if isDir.boolValue && item.contains("parakeet") {
                installed.insert(item)
            } else if item.hasPrefix("ggml-") && item.hasSuffix(".bin") {
                let modelName = item
                    .replacingOccurrences(of: "ggml-", with: "")
                    .replacingOccurrences(of: ".bin", with: "")
                installed.insert(modelName)
            }
        }

        installedModels = installed

        // Get currently selected model from config
        if let engine = ConfigManager.shared.getString("engine"), engine == "parakeet" {
            if let model = ConfigManager.shared.getString("parakeet.model") {
                selectedModel = model
            }
        } else {
            if let model = ConfigManager.shared.getString("whisper.model") {
                selectedModel = model
            }
        }
    }

    private func selectModel(_ name: String) {
        let isParakeet = name.contains("parakeet")

        if isParakeet {
            ConfigManager.shared.updateConfig(key: "engine", value: "\"parakeet\"")
            ConfigManager.shared.updateConfig(key: "model", value: "\"\(name)\"", section: "[parakeet]")
        } else {
            ConfigManager.shared.updateConfig(key: "engine", value: "\"whisper\"")
            ConfigManager.shared.updateConfig(key: "model", value: "\"\(name)\"", section: "[whisper]")
        }

        selectedModel = name
        needsRestart = true
    }

    private func downloadModel(_ name: String) {
        downloadingModel = name

        DispatchQueue.global().async {
            let result = VoxtypeCLI.run(["setup", "--download", "--model", name])

            DispatchQueue.main.async {
                downloadingModel = nil
                loadInstalledModels()

                if result.success {
                    selectModel(name)
                }
            }
        }
    }

    private func restartDaemon() {
        VoxtypeCLI.restartDaemon()
    }
}

struct ModelRowView: View {
    let model: ModelDefinition
    let isInstalled: Bool
    let isSelected: Bool
    let isDownloading: Bool
    let onSelect: () -> Void
    let onDownload: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            // Status icon
            statusIcon
                .frame(width: 20)

            // Model info
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(model.name)
                        .fontWeight(isSelected ? .semibold : .regular)
                    if isSelected {
                        Text("Active")
                            .font(.caption)
                            .foregroundColor(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Color.green)
                            .cornerRadius(4)
                    }
                }

                if isDownloading {
                    HStack(spacing: 8) {
                        ProgressView()
                            .scaleEffect(0.7)
                        Text("Downloading...")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                } else {
                    Text("\(model.size) - \(model.description)")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }

            Spacer()

            // Action button
            if isDownloading {
                // No button while downloading
            } else if isInstalled {
                if !isSelected {
                    Button("Select") {
                        onSelect()
                    }
                    .buttonStyle(.bordered)
                }
            } else {
                Button("Download") {
                    onDownload()
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private var statusIcon: some View {
        if isSelected {
            Image(systemName: "checkmark.circle.fill")
                .foregroundColor(.green)
        } else if isInstalled {
            Image(systemName: "checkmark.circle")
                .foregroundColor(.secondary)
        } else {
            Image(systemName: "arrow.down.circle")
                .foregroundColor(.blue)
        }
    }
}

struct ModelCategory {
    let name: String
    let description: String
    let models: [ModelDefinition]
}

struct ModelDefinition {
    let id: String
    let name: String
    let size: String
    let description: String
}
