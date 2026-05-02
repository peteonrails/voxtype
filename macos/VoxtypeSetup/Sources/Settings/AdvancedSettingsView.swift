import SwiftUI

struct AdvancedSettingsView: View {
    @State private var autoStartEnabled: Bool = false
    @State private var daemonRunning: Bool = false
    @State private var daemonStatus: String = "Unknown"

    var body: some View {
        Form {
            Section {
                HStack {
                    VStack(alignment: .leading) {
                        Text("Configuration File")
                        Text("~/Library/Application Support/voxtype/config.toml")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    Spacer()

                    Button("Open in Editor") {
                        openConfigFile()
                    }
                }

                HStack {
                    VStack(alignment: .leading) {
                        Text("Log Files")
                        Text("~/Library/Logs/voxtype/")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    Spacer()

                    Button("Open Folder") {
                        openLogsFolder()
                    }
                }

                HStack {
                    VStack(alignment: .leading) {
                        Text("Models Folder")
                        Text("~/Library/Application Support/voxtype/models/")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    Spacer()

                    Button("Open Folder") {
                        openModelsFolder()
                    }
                }
            } header: {
                Text("Files & Folders")
            }

            Section {
                Toggle("Start Voxtype at login", isOn: $autoStartEnabled)
                    .onChange(of: autoStartEnabled) { newValue in
                        toggleAutoStart(enabled: newValue)
                    }

                Text("Runs the Voxtype daemon automatically when you log in.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            } header: {
                Text("Auto-Start")
            }

            Section {
                HStack {
                    Circle()
                        .fill(daemonRunning ? Color.green : Color.red)
                        .frame(width: 10, height: 10)
                    Text("Status: \(daemonStatus)")

                    Spacer()

                    Button("Refresh") {
                        checkDaemonStatus()
                    }
                }

                if daemonRunning {
                    Button(action: restartDaemon) {
                        Label("Restart Daemon", systemImage: "arrow.clockwise")
                    }
                } else {
                    Button(action: startDaemon) {
                        Label("Start Daemon", systemImage: "play.fill")
                    }
                }

                Button(action: stopDaemon) {
                    Label("Stop Daemon", systemImage: "stop.fill")
                }
                .foregroundColor(.red)
                .disabled(!daemonRunning)

                Button(action: runSetupCheck) {
                    Label("Run Setup Check", systemImage: "checkmark.circle")
                }
            } header: {
                Text("Daemon")
            }

            Section {
                HStack {
                    Text("Version")
                    Spacer()
                    Text(getVersion())
                        .foregroundColor(.secondary)
                }

                Link(destination: URL(string: "https://github.com/peteonrails/voxtype")!) {
                    Label("View on GitHub", systemImage: "link")
                }

                Link(destination: URL(string: "https://voxtype.io")!) {
                    Label("Documentation", systemImage: "book")
                }
            } header: {
                Text("About")
            }
        }
        .formStyle(.grouped)
        .onAppear {
            checkAutoStartStatus()
            checkDaemonStatus()
        }
    }

    private func checkDaemonStatus() {
        let result = VoxtypeCLI.run(["status"])
        let status = result.output.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        if status == "idle" || status == "recording" || status == "transcribing" {
            daemonRunning = true
            daemonStatus = status.capitalized
        } else if status.contains("not running") || status.isEmpty || !result.success {
            daemonRunning = false
            daemonStatus = "Not Running"
        } else {
            daemonRunning = false
            daemonStatus = status.capitalized
        }
    }

    private func startDaemon() {
        VoxtypeCLI.run(["daemon"], wait: false)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            checkDaemonStatus()
        }
    }

    private func openConfigFile() {
        let path = NSHomeDirectory() + "/Library/Application Support/voxtype/config.toml"
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    private func openLogsFolder() {
        let path = NSHomeDirectory() + "/Library/Logs/voxtype"
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    private func openModelsFolder() {
        let path = NSHomeDirectory() + "/Library/Application Support/voxtype/models"
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    private func checkAutoStartStatus() {
        let plistPath = NSHomeDirectory() + "/Library/LaunchAgents/io.voxtype.daemon.plist"
        autoStartEnabled = FileManager.default.fileExists(atPath: plistPath)
    }

    private func toggleAutoStart(enabled: Bool) {
        if enabled {
            VoxtypeCLI.run(["setup", "launchd"])
        } else {
            VoxtypeCLI.run(["setup", "launchd", "--uninstall"])
        }
    }

    private func restartDaemon() {
        let task = Process()
        task.launchPath = "/bin/launchctl"
        task.arguments = ["kickstart", "-k", "gui/\(getuid())/io.voxtype.daemon"]
        try? task.run()
    }

    private func stopDaemon() {
        let task = Process()
        task.launchPath = "/bin/launchctl"
        task.arguments = ["stop", "io.voxtype.daemon"]
        try? task.run()
    }

    private func runSetupCheck() {
        // Open Terminal with setup check command
        let voxtype = VoxtypeCLI.binaryPath
        let script = """
            tell application "Terminal"
                do script "\(voxtype) setup check"
                activate
            end tell
            """
        if let appleScript = NSAppleScript(source: script) {
            var error: NSDictionary?
            appleScript.executeAndReturnError(&error)
        }
    }

    private func getVersion() -> String {
        let result = VoxtypeCLI.run(["--version"])
        return result.output.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
