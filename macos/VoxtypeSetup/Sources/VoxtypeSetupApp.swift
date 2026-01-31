import SwiftUI

@main
struct VoxtypeSetupApp: App {
    var body: some Scene {
        WindowGroup {
            SettingsView()
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 700, height: 500)
    }
}

/// Main settings view with sidebar navigation
struct SettingsView: View {
    @State private var selectedSection: SettingsSection = .general

    var body: some View {
        NavigationSplitView {
            List(SettingsSection.allCases, selection: $selectedSection) { section in
                Label(section.title, systemImage: section.icon)
                    .tag(section)
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 180, ideal: 200)
        } detail: {
            selectedSection.view
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding()
        }
        .navigationTitle("Voxtype Settings")
    }
}

/// Settings sections
enum SettingsSection: String, CaseIterable, Identifiable {
    case general
    case models
    case output
    case permissions
    case advanced

    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: return "General"
        case .models: return "Models"
        case .output: return "Output"
        case .permissions: return "Permissions"
        case .advanced: return "Advanced"
        }
    }

    var icon: String {
        switch self {
        case .general: return "gearshape"
        case .models: return "cpu"
        case .output: return "text.cursor"
        case .permissions: return "lock.shield"
        case .advanced: return "wrench.and.screwdriver"
        }
    }

    @ViewBuilder
    var view: some View {
        switch self {
        case .general: GeneralSettingsView()
        case .models: ModelsSettingsView()
        case .output: OutputSettingsView()
        case .permissions: PermissionsSettingsView()
        case .advanced: AdvancedSettingsView()
        }
    }
}
