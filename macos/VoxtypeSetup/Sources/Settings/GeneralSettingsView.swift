import SwiftUI
import AppKit

struct GeneralSettingsView: View {
    @State private var selectedEngine: String = "parakeet"
    @State private var hotkeyMode: String = "push_to_talk"
    @State private var hotkey: String = "RIGHTALT"
    @State private var daemonRunning: Bool = false
    @State private var menubarRunning: Bool = false
    @State private var needsRestart: Bool = false

    var body: some View {
        Form {
            if needsRestart {
                Section {
                    HStack {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundColor(.orange)
                        Text("Engine changed. Restart daemon to apply.")
                        Spacer()
                        Button("Restart Now") {
                            restartDaemon()
                            needsRestart = false
                        }
                        .buttonStyle(.borderedProminent)
                    }
                }
            }

            Section {
                Picker("Transcription Engine", selection: $selectedEngine) {
                    Text("Parakeet (Fast)").tag("parakeet")
                    Text("Whisper").tag("whisper")
                }
                .onChange(of: selectedEngine) { newValue in
                    ConfigManager.shared.updateConfig(key: "engine", value: "\"\(newValue)\"")
                    needsRestart = true
                }

                Text("Parakeet is faster and recommended for most users.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Engine")
            }

            Section {
                Picker("Hotkey", selection: $hotkey) {
                    Text("Right Option (⌥)").tag("RIGHTALT")
                    Text("Right Command (⌘)").tag("RIGHTMETA")
                    Text("Right Control (⌃)").tag("RIGHTCTRL")
                    Text("F13").tag("F13")
                    Text("F14").tag("F14")
                    Text("F15").tag("F15")
                }
                .onChange(of: hotkey) { newValue in
                    ConfigManager.shared.updateConfig(key: "key", value: "\"\(newValue)\"", section: "[hotkey]")
                    needsRestart = true
                }

                Picker("Mode", selection: $hotkeyMode) {
                    Text("Push-to-Talk (hold to record)").tag("push_to_talk")
                    Text("Toggle (press to start/stop)").tag("toggle")
                }
                .onChange(of: hotkeyMode) { newValue in
                    ConfigManager.shared.updateConfig(key: "mode", value: "\"\(newValue)\"", section: "[hotkey]")
                    needsRestart = true
                }
            } header: {
                Text("Hotkey")
            }

            Section {
                HStack {
                    Circle()
                        .fill(daemonRunning ? Color.green : Color.red)
                        .frame(width: 10, height: 10)
                    Text(daemonRunning ? "Daemon is running" : "Daemon is not running")

                    Spacer()

                    if daemonRunning {
                        Button("Restart") {
                            restartDaemon()
                        }
                    } else {
                        Button("Start") {
                            startDaemon()
                        }
                    }
                }
            } header: {
                Text("Daemon Status")
            }

            Section {
                Toggle("Show in Menu Bar", isOn: $menubarRunning)
                    .onChange(of: menubarRunning) { newValue in
                        if newValue {
                            launchMenubar()
                        } else {
                            quitMenubar()
                        }
                    }

                Text("Display a status icon in the menu bar for quick access.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Menu Bar")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadSettings()
            checkDaemonStatus()
            checkMenubarStatus()
        }
    }

    private func loadSettings() {
        if let engine = ConfigManager.shared.getString("engine") {
            selectedEngine = engine
        }

        if let key = ConfigManager.shared.getString("hotkey.key") {
            hotkey = key
        }

        if let mode = ConfigManager.shared.getString("hotkey.mode") {
            hotkeyMode = mode
        }
    }

    private func checkDaemonStatus() {
        let result = VoxtypeCLI.run(["status"])
        let status = result.output.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        daemonRunning = (status == "idle" || status == "recording" || status == "transcribing")
    }

    private func startDaemon() {
        _ = VoxtypeCLI.run(["daemon"], wait: false)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            checkDaemonStatus()
        }
    }

    private func restartDaemon() {
        VoxtypeCLI.restartDaemon {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                self.checkDaemonStatus()
            }
        }
    }

    private func checkMenubarStatus() {
        let task = Process()
        task.launchPath = "/usr/bin/pgrep"
        task.arguments = ["-x", "VoxtypeMenubar"]
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        try? task.run()
        task.waitUntilExit()
        menubarRunning = (task.terminationStatus == 0)
    }

    private func launchMenubar() {
        let menubarPath = "/Applications/Voxtype.app/Contents/MacOS/VoxtypeMenubar.app"
        if FileManager.default.fileExists(atPath: menubarPath) {
            NSWorkspace.shared.open(URL(fileURLWithPath: menubarPath))
        }
    }

    private func quitMenubar() {
        let task = Process()
        task.launchPath = "/usr/bin/pkill"
        task.arguments = ["-x", "VoxtypeMenubar"]
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        try? task.run()
        task.waitUntilExit()
    }
}
