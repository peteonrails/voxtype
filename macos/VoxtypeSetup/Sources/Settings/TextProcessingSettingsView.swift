import SwiftUI

struct TextProcessingSettingsView: View {
    @State private var spokenPunctuation: Bool = false
    @State private var replacements: [(key: String, value: String)] = []
    @State private var newKey: String = ""
    @State private var newValue: String = ""

    var body: some View {
        Form {
            Section {
                Toggle("Enable Spoken Punctuation", isOn: $spokenPunctuation)
                    .onChange(of: spokenPunctuation) { newValue in
                        ConfigManager.shared.updateConfig(key: "spoken_punctuation", value: newValue ? "true" : "false", section: "[text]")
                    }

                VStack(alignment: .leading, spacing: 4) {
                    Text("Convert spoken words to punctuation marks:")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Text("• \"period\" → \".\"")
                    Text("• \"comma\" → \",\"")
                    Text("• \"question mark\" → \"?\"")
                    Text("• \"exclamation point\" → \"!\"")
                    Text("• \"new line\" → newline")
                }
                .font(.caption)
                .foregroundColor(.secondary)
            } header: {
                Text("Spoken Punctuation")
            }

            Section {
                if replacements.isEmpty {
                    Text("No word replacements configured")
                        .foregroundColor(.secondary)
                } else {
                    ForEach(Array(replacements.enumerated()), id: \.offset) { index, replacement in
                        HStack {
                            Text("\"\(replacement.key)\"")
                            Image(systemName: "arrow.right")
                                .foregroundColor(.secondary)
                            Text("\"\(replacement.value)\"")
                            Spacer()
                            Button(role: .destructive) {
                                removeReplacement(at: index)
                            } label: {
                                Image(systemName: "trash")
                            }
                            .buttonStyle(.borderless)
                        }
                    }
                }

                Divider()

                HStack {
                    TextField("From", text: $newKey)
                        .textFieldStyle(.roundedBorder)
                    Image(systemName: "arrow.right")
                        .foregroundColor(.secondary)
                    TextField("To", text: $newValue)
                        .textFieldStyle(.roundedBorder)
                    Button("Add") {
                        addReplacement()
                    }
                    .disabled(newKey.isEmpty || newValue.isEmpty)
                }

                Text("Example: \"vox type\" → \"voxtype\"")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Word Replacements")
            } footer: {
                Text("Replacements are case-insensitive and applied after transcription.")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
        }
    }

    private func loadSettings() {
        let config = ConfigManager.shared.readConfig()

        if let sp = config["text.spoken_punctuation"] {
            spokenPunctuation = sp == "true"
        }

        // Load replacements - they're stored as a TOML table
        // For now, we'll parse them from the raw config file
        loadReplacements()
    }

    private func loadReplacements() {
        let configPath = NSHomeDirectory() + "/Library/Application Support/voxtype/config.toml"
        guard let content = try? String(contentsOfFile: configPath, encoding: .utf8) else {
            return
        }

        var inReplacementsSection = false
        var loaded: [(key: String, value: String)] = []

        for line in content.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed == "[text.replacements]" {
                inReplacementsSection = true
                continue
            }

            if trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                inReplacementsSection = false
                continue
            }

            if inReplacementsSection && trimmed.contains("=") && !trimmed.hasPrefix("#") {
                let parts = trimmed.components(separatedBy: "=")
                if parts.count >= 2 {
                    let key = parts[0].trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "\"", with: "")
                    let value = parts.dropFirst().joined(separator: "=").trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "\"", with: "")
                    loaded.append((key: key, value: value))
                }
            }
        }

        replacements = loaded
    }

    private func addReplacement() {
        guard !newKey.isEmpty && !newValue.isEmpty else { return }

        replacements.append((key: newKey, value: newValue))
        saveReplacements()

        newKey = ""
        newValue = ""
    }

    private func removeReplacement(at index: Int) {
        replacements.remove(at: index)
        saveReplacements()
    }

    private func saveReplacements() {
        let configPath = NSHomeDirectory() + "/Library/Application Support/voxtype/config.toml"
        guard var content = try? String(contentsOfFile: configPath, encoding: .utf8) else {
            return
        }

        // Remove existing [text.replacements] section
        var lines = content.components(separatedBy: .newlines)
        var newLines: [String] = []
        var inReplacementsSection = false

        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed == "[text.replacements]" {
                inReplacementsSection = true
                continue
            }

            if inReplacementsSection && trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                inReplacementsSection = false
            }

            if !inReplacementsSection {
                newLines.append(line)
            }
        }

        // Add new [text.replacements] section
        if !replacements.isEmpty {
            newLines.append("")
            newLines.append("[text.replacements]")
            for r in replacements {
                newLines.append("\"\(r.key)\" = \"\(r.value)\"")
            }
        }

        content = newLines.joined(separator: "\n")
        try? content.write(toFile: configPath, atomically: true, encoding: .utf8)
    }
}
