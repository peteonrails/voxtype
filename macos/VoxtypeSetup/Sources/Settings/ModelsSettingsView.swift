import SwiftUI

struct ModelsSettingsView: View {
    @State private var installedModels: [ModelInfo] = []
    @State private var selectedModel: String = ""
    @State private var isDownloading: Bool = false
    @State private var downloadProgress: String = ""

    var body: some View {
        Form {
            Section {
                if installedModels.isEmpty {
                    Text("No models installed")
                        .foregroundColor(.secondary)
                } else {
                    ForEach(installedModels, id: \.name) { model in
                        HStack {
                            VStack(alignment: .leading) {
                                Text(model.name)
                                    .fontWeight(model.name == selectedModel ? .semibold : .regular)
                                Text(model.size)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }

                            Spacer()

                            if model.name == selectedModel {
                                Image(systemName: "checkmark.circle.fill")
                                    .foregroundColor(.green)
                            } else {
                                Button("Select") {
                                    selectModel(model.name)
                                }
                            }
                        }
                        .padding(.vertical, 4)
                    }
                }
            } header: {
                Text("Installed Models")
            }

            Section {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Parakeet (Recommended)")
                        .font(.headline)

                    HStack {
                        Button("Download parakeet-tdt-0.6b-v3-int8") {
                            downloadModel("parakeet-tdt-0.6b-v3-int8")
                        }
                        .disabled(isDownloading)

                        Text("~640 MB")
                            .foregroundColor(.secondary)
                    }
                }

                Divider()

                VStack(alignment: .leading, spacing: 12) {
                    Text("Whisper Models")
                        .font(.headline)

                    HStack {
                        Button("Download base.en") {
                            downloadModel("base.en")
                        }
                        .disabled(isDownloading)

                        Text("~142 MB - Good balance")
                            .foregroundColor(.secondary)
                    }

                    HStack {
                        Button("Download small.en") {
                            downloadModel("small.en")
                        }
                        .disabled(isDownloading)

                        Text("~466 MB - Better accuracy")
                            .foregroundColor(.secondary)
                    }

                    HStack {
                        Button("Download large-v3-turbo") {
                            downloadModel("large-v3-turbo")
                        }
                        .disabled(isDownloading)

                        Text("~1.6 GB - Best quality")
                            .foregroundColor(.secondary)
                    }
                }

                if isDownloading {
                    HStack {
                        ProgressView()
                            .scaleEffect(0.8)
                        Text(downloadProgress)
                            .foregroundColor(.secondary)
                    }
                }
            } header: {
                Text("Download Models")
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

        var models: [ModelInfo] = []

        for item in contents {
            let path = modelsDir + "/" + item

            var isDir: ObjCBool = false
            FileManager.default.fileExists(atPath: path, isDirectory: &isDir)

            if isDir.boolValue && item.contains("parakeet") {
                // Parakeet model directory
                let size = getDirectorySize(path)
                models.append(ModelInfo(name: item, size: formatSize(size), isParakeet: true))
            } else if item.hasPrefix("ggml-") && item.hasSuffix(".bin") {
                // Whisper model file
                if let attrs = try? FileManager.default.attributesOfItem(atPath: path),
                   let size = attrs[.size] as? Int64 {
                    let modelName = item
                        .replacingOccurrences(of: "ggml-", with: "")
                        .replacingOccurrences(of: ".bin", with: "")
                    models.append(ModelInfo(name: modelName, size: formatSize(size), isParakeet: false))
                }
            }
        }

        installedModels = models

        // Get currently selected model from config
        let config = readConfig()
        if let engine = config["engine"]?.replacingOccurrences(of: "\"", with: ""),
           engine == "parakeet" {
            if let model = config["parakeet.model"]?.replacingOccurrences(of: "\"", with: "") {
                selectedModel = model
            }
        } else {
            if let model = config["whisper.model"]?.replacingOccurrences(of: "\"", with: "") {
                selectedModel = model
            }
        }
    }

    private func selectModel(_ name: String) {
        let isParakeet = name.contains("parakeet")

        if isParakeet {
            updateConfig(key: "engine", value: "\"parakeet\"")
            updateConfig(key: "model", value: "\"\(name)\"", section: "[parakeet]")
        } else {
            updateConfig(key: "engine", value: "\"whisper\"")
            updateConfig(key: "model", value: "\"\(name)\"", section: "[whisper]")
        }

        selectedModel = name
    }

    private func downloadModel(_ name: String) {
        isDownloading = true
        downloadProgress = "Downloading \(name)..."

        DispatchQueue.global().async {
            let result = VoxtypeCLI.run(["setup", "--download", "--model", name])

            DispatchQueue.main.async {
                isDownloading = false
                downloadProgress = ""
                loadInstalledModels()

                if result.success {
                    selectModel(name)
                }
            }
        }
    }

    private func getDirectorySize(_ path: String) -> Int64 {
        var size: Int64 = 0
        if let enumerator = FileManager.default.enumerator(atPath: path) {
            while let file = enumerator.nextObject() as? String {
                let filePath = path + "/" + file
                if let attrs = try? FileManager.default.attributesOfItem(atPath: filePath),
                   let fileSize = attrs[.size] as? Int64 {
                    size += fileSize
                }
            }
        }
        return size
    }

    private func formatSize(_ bytes: Int64) -> String {
        let mb = Double(bytes) / 1_000_000
        if mb >= 1000 {
            return String(format: "%.1f GB", mb / 1000)
        }
        return String(format: "%.0f MB", mb)
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

struct ModelInfo {
    let name: String
    let size: String
    let isParakeet: Bool
}
