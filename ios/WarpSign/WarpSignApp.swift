import SwiftUI

@main
struct WarpSignApp: App {
    @State private var library = LibraryStore()
    @State private var account = AccountStore()
    @State private var sources = SourcesStore()
    @State private var inspection = InspectionModel()
    @State private var tab = AppTab.library

    enum AppTab: Hashable {
        case library, sources, settings
    }

    var body: some Scene {
        WindowGroup {
            TabView(selection: $tab) {
                LibraryView()
                    .tabItem { Label("Library", systemImage: "square.stack.3d.up.fill") }
                    .tag(AppTab.library)
                SourcesView()
                    .tabItem { Label("Sources", systemImage: "globe") }
                    .tag(AppTab.sources)
                SettingsView()
                    .tabItem { Label("Settings", systemImage: "gearshape") }
                    .tag(AppTab.settings)
                    .badge(account.needsBackup ? Text("!") : nil)
            }
            .environment(library)
            .environment(account)
            .environment(sources)
            .environment(inspection)
            .task { await account.restore() }
            // IPAs and profiles opened in WarpSign from Files or the share sheet
            .onOpenURL { url in
                Task {
                    switch url.pathExtension.lowercased() {
                    case "mobileprovision":
                        tab = .settings
                        await inspection.addProfile(url)
                    case "ipa":
                        tab = .library
                        await library.importFile(url)
                    default:
                        break
                    }
                }
            }
        }
    }
}
