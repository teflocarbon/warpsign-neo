import SwiftUI

@main
struct WarpSignApp: App {
    @State private var library = Demo.isOn ? Demo.library() : LibraryStore()
    @State private var account = Demo.isOn ? Demo.account() : AccountStore()
    @State private var sources = Demo.isOn ? Demo.sources() : SourcesStore()
    @State private var inspection = InspectionModel()
    @State private var device = DeviceStore()
    @State private var tab = Demo.isOn ? Demo.tab : AppTab.library

    enum AppTab: Hashable {
        case library, sources, settings
    }

    var body: some Scene {
        WindowGroup {
            TabView(selection: $tab) {
                libraryTab
                    .tabItem { Label("Library", systemImage: "square.stack.3d.up.fill") }
                    .tag(AppTab.library)
                SourcesView()
                    .tabItem { Label("Sources", systemImage: "globe") }
                    .tag(AppTab.sources)
                settingsTab
                    .tabItem { Label("Settings", systemImage: "gearshape") }
                    .tag(AppTab.settings)
                    .badge(account.needsBackup ? Text("!") : nil)
            }
            .environment(library)
            .environment(account)
            .environment(sources)
            .environment(inspection)
            .environment(device)
            .task { await account.restore() }
            // IPAs, profiles and pairing files opened in WarpSign from Files or the share sheet
            .onOpenURL { url in
                Task {
                    switch url.pathExtension.lowercased() {
                    case "mobileprovision":
                        tab = .settings
                        await inspection.addProfile(url)
                    case "mobiledevicepairing":
                        tab = .settings
                        device.importPairingFile(url)
                        await device.check()
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

    /// The Library tab, or a screen inside it for screenshots
    @ViewBuilder
    private var libraryTab: some View {
        if Demo.isOn, let app = library.apps.first {
            switch Demo.screen {
            case .signOptions:
                NavigationStack { SignOptionsView(app: app) }
            case .signing:
                let demo = Demo.signingResult
                NavigationStack { SigningView(demoApp: app, result: demo.0, copy: demo.1, steps: demo.2) }
            case .install:
                NavigationStack { InstallView(demoName: app.displayName, steps: Demo.installSteps) }
            default:
                LibraryView()
            }
        } else {
            LibraryView()
        }
    }

    /// The Settings tab, or a screen inside it for screenshots
    @ViewBuilder
    private var settingsTab: some View {
        if Demo.isOn && Demo.screen == .installing {
            NavigationStack { InstallingView() }
        } else if Demo.isOn && Demo.screen == .signIn {
            SignInView()
        } else {
            SettingsView()
        }
    }
}

extension Demo {
    static var tab: WarpSignApp.AppTab {
        switch screen {
        case .library, .appDetail, .signOptions, .signing, .install: .library
        case .sources, .sourceDetail: .sources
        case .settings, .installing, .signIn: .settings
        }
    }
}
