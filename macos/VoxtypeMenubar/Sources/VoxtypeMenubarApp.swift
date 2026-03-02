import SwiftUI

@main
struct VoxtypeMenubarApp: App {
    @StateObject private var statusMonitor = VoxtypeStatusMonitor()

    var body: some Scene {
        MenuBarExtra {
            MenuBarView()
                .environmentObject(statusMonitor)
        } label: {
            Image(systemName: statusMonitor.iconName)
                .symbolRenderingMode(.hierarchical)
        }
        .menuBarExtraStyle(.menu)
    }
}
