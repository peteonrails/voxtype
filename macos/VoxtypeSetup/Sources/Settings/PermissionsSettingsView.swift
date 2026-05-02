import SwiftUI

struct PermissionsSettingsView: View {
    @State private var microphoneGranted: Bool = false
    @State private var inputMonitoringGranted: Bool = false
    @State private var accessibilityGranted: Bool = false

    var body: some View {
        Form {
            Section {
                PermissionRow(
                    title: "Microphone",
                    description: "Required to capture your voice for transcription",
                    icon: "mic.fill",
                    isGranted: microphoneGranted
                ) {
                    openSystemPreferences("Privacy_Microphone")
                }

                PermissionRow(
                    title: "Input Monitoring",
                    description: "Required for global hotkey detection",
                    icon: "keyboard",
                    isGranted: inputMonitoringGranted
                ) {
                    openSystemPreferences("Privacy_ListenEvent")
                }

                PermissionRow(
                    title: "Accessibility",
                    description: "Required to type transcribed text into applications",
                    icon: "hand.raised.fill",
                    isGranted: accessibilityGranted
                ) {
                    openSystemPreferences("Privacy_Accessibility")
                }
            } header: {
                Text("Required Permissions")
            } footer: {
                Text("Click \"Open Settings\" to grant each permission. You may need to add Voxtype manually.")
            }

            Section {
                Button(action: checkPermissions) {
                    Label("Refresh Permission Status", systemImage: "arrow.clockwise")
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            checkPermissions()
        }
    }

    private func checkPermissions() {
        // Check microphone permission
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            microphoneGranted = true
        default:
            microphoneGranted = false
        }

        // Input monitoring and accessibility are harder to check programmatically
        // We use a heuristic: try to see if voxtype status works
        let result = VoxtypeCLI.run(["status"])
        let status = result.output.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        // If daemon is running and responding, permissions are likely granted
        if status == "idle" || status == "recording" || status == "transcribing" {
            inputMonitoringGranted = true
            accessibilityGranted = true
        }
    }

    private func openSystemPreferences(_ pane: String) {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)")!
        NSWorkspace.shared.open(url)
    }
}

struct PermissionRow: View {
    let title: String
    let description: String
    let icon: String
    let isGranted: Bool
    let openSettings: () -> Void

    var body: some View {
        HStack {
            Image(systemName: icon)
                .frame(width: 24)
                .foregroundColor(.accentColor)

            VStack(alignment: .leading) {
                Text(title)
                    .fontWeight(.medium)
                Text(description)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Spacer()

            if isGranted {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundColor(.green)
            } else {
                Button("Open Settings") {
                    openSettings()
                }
                .buttonStyle(.bordered)
            }
        }
        .padding(.vertical, 4)
    }
}

import AVFoundation
