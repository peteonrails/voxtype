import Foundation
import AVFoundation
import AppKit

/// Checks and requests macOS permissions required by Voxtype
class PermissionChecker: ObservableObject {
    static let shared = PermissionChecker()

    @Published var hasMicrophoneAccess: Bool = false
    @Published var hasAccessibilityAccess: Bool = false
    @Published var hasInputMonitoringAccess: Bool = false

    private init() {
        refresh()
    }

    /// Refresh all permission states
    func refresh() {
        checkMicrophoneAccess()
        checkAccessibilityAccess()
        checkInputMonitoringAccess()
    }

    // MARK: - Microphone

    private func checkMicrophoneAccess() {
        // Check confirmation from user (permission is for Voxtype.app, not this app)
        hasMicrophoneAccess = UserDefaults.standard.bool(forKey: "microphoneConfirmed")
    }

    func openMicrophoneSettings() {
        // Use osascript to open Microphone privacy settings directly
        let script = """
        tell application "System Settings"
            activate
            reveal anchor "Privacy_Microphone" of pane id "com.apple.settings.PrivacySecurity.extension"
        end tell
        """

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script]
        try? process.run()
    }

    func confirmMicrophoneAccess() {
        UserDefaults.standard.set(true, forKey: "microphoneConfirmed")
        hasMicrophoneAccess = true
    }

    // MARK: - Accessibility

    private func checkAccessibilityAccess() {
        // Check if THIS app (setup wizard) is trusted
        // Note: Main Voxtype.app permission must be confirmed manually
        hasAccessibilityAccess = UserDefaults.standard.bool(forKey: "accessibilityConfirmed")
    }

    func requestAccessibilityAccess() {
        // Open System Settings to Accessibility
        openAccessibilitySettings()
    }

    func confirmAccessibilityAccess() {
        UserDefaults.standard.set(true, forKey: "accessibilityConfirmed")
        hasAccessibilityAccess = true
    }

    func openAccessibilitySettings() {
        // Use osascript to open Accessibility directly
        let script = """
        tell application "System Settings"
            activate
            reveal anchor "Privacy_Accessibility" of pane id "com.apple.settings.PrivacySecurity.extension"
        end tell
        """

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script]
        try? process.run()
    }

    // MARK: - Input Monitoring

    private func checkInputMonitoringAccess() {
        // Check confirmation from user
        hasInputMonitoringAccess = UserDefaults.standard.bool(forKey: "inputMonitoringConfirmed")
    }

    func openInputMonitoringSettings() {
        // Use osascript to open Input Monitoring directly
        let script = """
        tell application "System Settings"
            activate
            reveal anchor "Privacy_ListenEvent" of pane id "com.apple.settings.PrivacySecurity.extension"
        end tell
        """

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script]
        try? process.run()
    }

    func confirmInputMonitoringAccess() {
        UserDefaults.standard.set(true, forKey: "inputMonitoringConfirmed")
        hasInputMonitoringAccess = true
    }

    // MARK: - Notifications (optional)

    func openNotificationSettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.notifications")!
        NSWorkspace.shared.open(url)
    }
}
