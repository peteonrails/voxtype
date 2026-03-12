import Foundation

/// Centralized config file management
class ConfigManager {
    static let shared = ConfigManager()

    private let configPath: String

    private init() {
        configPath = NSHomeDirectory() + "/Library/Application Support/voxtype/config.toml"
    }

    /// Read config file and return key-value pairs
    /// Keys are in the format "section.key" (e.g., "hotkey.key", "whisper.model")
    func readConfig() -> [String: String] {
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

    /// Update a config value within a specific section
    /// - Parameters:
    ///   - key: The key name (without section prefix)
    ///   - value: The new value (including quotes if string)
    ///   - section: Optional section like "[hotkey]" - if provided, only updates the key within that section
    func updateConfig(key: String, value: String, section: String? = nil) {
        guard let content = try? String(contentsOfFile: configPath, encoding: .utf8) else {
            return
        }

        var lines = content.components(separatedBy: .newlines)
        let targetSection = section?.trimmingCharacters(in: CharacterSet(charactersIn: "[]")) ?? ""
        var currentSection = ""
        var foundAndReplaced = false

        for i in 0..<lines.count {
            let trimmed = lines[i].trimmingCharacters(in: .whitespaces)

            // Track current section
            if trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                currentSection = String(trimmed.dropFirst().dropLast())
                continue
            }

            // Skip comments
            if trimmed.hasPrefix("#") {
                continue
            }

            // Check if this line has our key
            if trimmed.hasPrefix("\(key) ") || trimmed.hasPrefix("\(key)=") {
                // Check if we're in the right section
                let inCorrectSection = (section == nil && currentSection.isEmpty) ||
                                       (currentSection == targetSection)

                if inCorrectSection {
                    lines[i] = "\(key) = \(value)"
                    foundAndReplaced = true
                    break
                }
            }
        }

        // If not found and we have a section, add it
        if !foundAndReplaced, let section = section {
            let newContent = addKeyToSection(content: lines.joined(separator: "\n"), section: section, key: key, value: value)
            try? newContent.write(toFile: configPath, atomically: true, encoding: .utf8)
            return
        }

        let newContent = lines.joined(separator: "\n")
        try? newContent.write(toFile: configPath, atomically: true, encoding: .utf8)
    }

    /// Add a key to a specific section in the config
    private func addKeyToSection(content: String, section: String, key: String, value: String) -> String {
        var lines = content.components(separatedBy: .newlines)
        var sectionIndex: Int? = nil

        // Find the section
        for (index, line) in lines.enumerated() {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed == section {
                sectionIndex = index
                break
            }
        }

        if let sectionIndex = sectionIndex {
            // Find the end of this section (next section or end of file)
            var insertIndex = sectionIndex + 1
            for i in (sectionIndex + 1)..<lines.count {
                let trimmed = lines[i].trimmingCharacters(in: .whitespaces)
                if trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                    break
                }
                insertIndex = i + 1
            }
            lines.insert("\(key) = \(value)", at: insertIndex)
        } else {
            // Section doesn't exist, add it at the end
            lines.append("")
            lines.append(section)
            lines.append("\(key) = \(value)")
        }

        return lines.joined(separator: "\n")
    }

    /// Get a string value from config, removing quotes
    func getString(_ key: String) -> String? {
        readConfig()[key]?.replacingOccurrences(of: "\"", with: "")
    }

    /// Get a boolean value from config
    func getBool(_ key: String) -> Bool? {
        guard let value = readConfig()[key] else { return nil }
        return value == "true"
    }

    /// Get an integer value from config
    func getInt(_ key: String) -> Int? {
        guard let value = readConfig()[key] else { return nil }
        return Int(value)
    }
}
