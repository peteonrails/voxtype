import Foundation
import Combine

/// Monitors voxtype daemon state by watching the state file
class VoxtypeStatusMonitor: ObservableObject {
    @Published var state: VoxtypeState = .stopped
    @Published var daemonRunning: Bool = false

    private var timer: Timer?
    private let stateFilePath = "/tmp/voxtype/state"

    var iconName: String {
        switch state {
        case .idle:
            return "mic.fill"
        case .recording:
            return "mic.badge.plus"
        case .transcribing:
            return "ellipsis.circle.fill"
        case .stopped:
            return "mic.slash.fill"
        }
    }

    var statusText: String {
        switch state {
        case .idle:
            return "Ready"
        case .recording:
            return "Recording..."
        case .transcribing:
            return "Transcribing..."
        case .stopped:
            return "Daemon not running"
        }
    }

    init() {
        startMonitoring()
    }

    deinit {
        stopMonitoring()
    }

    func startMonitoring() {
        // Check immediately
        updateState()

        // Then poll every 500ms
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.updateState()
        }
    }

    func stopMonitoring() {
        timer?.invalidate()
        timer = nil
    }

    private func updateState() {
        // Check if daemon is running
        daemonRunning = isDaemonRunning()

        if !daemonRunning {
            state = .stopped
            return
        }

        // Read state file
        guard let content = try? String(contentsOfFile: stateFilePath, encoding: .utf8) else {
            state = .stopped
            return
        }

        let trimmed = content.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch trimmed {
        case "idle":
            state = .idle
        case "recording":
            state = .recording
        case "transcribing":
            state = .transcribing
        default:
            state = .stopped
        }
    }

    private func isDaemonRunning() -> Bool {
        // First check if launchd service is running
        let launchctlTask = Process()
        launchctlTask.launchPath = "/bin/launchctl"
        launchctlTask.arguments = ["list", "io.voxtype.daemon"]
        launchctlTask.standardOutput = FileHandle.nullDevice
        launchctlTask.standardError = FileHandle.nullDevice

        do {
            try launchctlTask.run()
            launchctlTask.waitUntilExit()
            if launchctlTask.terminationStatus == 0 {
                return true
            }
        } catch {}

        // Fall back to checking if daemon process is running via PID file
        let pidPath = "/tmp/voxtype/pid"
        guard let pidString = try? String(contentsOfFile: pidPath, encoding: .utf8),
              let pid = Int32(pidString.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            return false
        }

        // Check if process with this PID exists
        return kill(pid, 0) == 0
    }
}

enum VoxtypeState {
    case idle
    case recording
    case transcribing
    case stopped
}
