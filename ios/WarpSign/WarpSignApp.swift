import SwiftUI

@main
struct WarpSignApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @Environment(\.scenePhase) private var scenePhase
    @State private var library: LibraryStore
    @State private var account: AccountStore
    @State private var sources: SourcesStore
    @State private var inspection = InspectionModel()
    @State private var device: DeviceStore
    @State private var tab = Demo.isOn ? Demo.tab : AppTab.library

    init() {
        let library = Demo.isOn ? Demo.library() : LibraryStore()
        let account = Demo.isOn ? Demo.account() : AccountStore()
        let sources = Demo.isOn ? Demo.sources() : SourcesStore()
        let device = DeviceStore()
        _library = State(initialValue: library)
        _account = State(initialValue: account)
        _sources = State(initialValue: sources)
        _device = State(initialValue: device)
        // Background launches update apps without any screen
        Updater.shared.attach(library: library, account: account, sources: sources, device: device)
        DownloadManager.shared.attach(library: library)
    }

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
            .modifier(PendingInstallSheet())
            .environment(library)
            .environment(account)
            .environment(sources)
            .environment(inspection)
            .environment(device)
            .task {
                await account.restore()
                await Updater.shared.checkIfDue()
            }
            .onChange(of: scenePhase) { _, phase in
                switch phase {
                case .active:
                    Task { await Updater.shared.checkIfDue() }
                case .background:
                    BackgroundUpdates.scheduleRefresh()
                    if Updater.shared.automatic && !Updater.shared.automaticIds.isEmpty {
                        BackgroundUpdates.scheduleUpdates()
                    }
                default:
                    break
                }
            }
            // IPAs, profiles and pairing files opened in WarpSign from Files or the share sheet
            .onOpenURL { url in
                // altstore://, sidestore://, warpsign:// source and install links
                if !url.isFileURL, let link = parseStoreLink(text: url.absoluteString) {
                    switch link {
                    case .source(let source):
                        tab = .sources
                        sources.proposed = source
                    case .install(let ipa):
                        tab = .library
                        do {
                            try DownloadManager.shared.download(link: ipa)
                        } catch {
                            library.importError = error.userMessage
                        }
                    }
                    return
                }
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
