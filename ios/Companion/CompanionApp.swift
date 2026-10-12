import SwiftUI
import UniformTypeIdentifiers

/// WarpSign Companion: installs an already-signed IPA on this device with a
/// pairing file, as WarpSign does. A separate app, with its own VPN, so it
/// can install WarpSign over WarpSign: WarpSign can't install over itself,
/// as iOS stops it, and what's feeding the install, partway through.
@main
struct CompanionApp: App {
    @State private var store = CompanionStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(store)
                .onOpenURL { url in store.open(url) }
        }
    }
}

struct ContentView: View {
    @Environment(CompanionStore.self) private var store
    @State private var importing: Kind?

    private enum Kind {
        case ipa, pairing
    }

    private static let ipaType = UTType("com.apple.itunes.ipa") ?? UTType(filenameExtension: "ipa") ?? .data
    private static let pairingTypes: [UTType] = [
        UTType(filenameExtension: "mobiledevicepairing") ?? .data,
        .propertyList,
    ]

    var body: some View {
        NavigationStack {
            List {
                appSection
                if let error = store.error {
                    Section { ErrorRow(message: error) }
                }
                setupSections
                Section {
                    LabeledContent("Companion", value: BuildInfo.versionText)
                    if let commit = BuildInfo.commit {
                        LabeledContent("Commit", value: commit)
                    }
                } header: {
                    Text("About")
                }
            }
            .navigationTitle("Companion")
            .fileImporter(
                isPresented: Binding(get: { importing != nil }, set: { if !$0 { importing = nil } }),
                // Any file, checked after: types depend on which apps declared them
                allowedContentTypes: [.data]
            ) { result in
                guard case .success(let url) = result else { return }
                // By extension: .ipa, or a pairing file
                store.open(url)
            }
            .task { await store.tunnel.refresh() }
        }
    }

    @ViewBuilder
    private var appSection: some View {
        Section {
            if let app = store.picked {
                HStack(spacing: 12) {
                    AppIconView(image: app.icon)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(app.name).font(.headline)
                        Text([app.versionText, app.bundleId].compactMap { $0 }.joined(separator: " · "))
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                if app.isCompanion {
                    Label("This is Companion itself, which can't install over itself. Install it with WarpSign.", systemImage: "nosign")
                        .foregroundStyle(.secondary)
                } else {
                    NavigationLink {
                        InstallScreen(app: app)
                    } label: {
                        Label("Install on This Device", systemImage: "arrow.down.app").font(.headline)
                    }
                    .disabled(!store.hasPairingFile)
                }
            }
            Button(store.picked == nil ? "Choose an IPA…" : "Choose Another IPA…", systemImage: "doc.badge.plus") {
                importing = .ipa
            }
        } header: {
            Text("App")
        } footer: {
            Text("For apps that are already signed, such as WarpSign itself: sign it in WarpSign, then Share Signed IPA → Companion.")
        }
    }

    @ViewBuilder
    private var setupSections: some View {
        Section {
            if store.hasPairingFile {
                Label("Pairing file added", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                if let udid = store.udid {
                    Text(udid).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Button("Replace Pairing File…") { importing = .pairing }
            } else {
                Button("Add Pairing File…", systemImage: "doc.badge.plus") { importing = .pairing }
            }
        } header: {
            Text("Pairing File")
        } footer: {
            Text("The same file WarpSign uses (from `warpsign pair`). Companion keeps its own copy in its Keychain.")
        }

        Section {
            switch store.tunnel.status {
            case .unknown, .connecting:
                LabeledContent("Companion VPN") { ProgressView() }
            case .unavailable(let reason):
                Text(reason).font(.footnote).foregroundStyle(.secondary)
            case .off:
                LabeledContent("Companion VPN", value: "Off")
            case .on:
                LabeledContent("Companion VPN") {
                    HStack(spacing: 6) {
                        Image(systemName: "checkmark.circle.fill")
                        Text("On")
                    }
                    .foregroundStyle(.green)
                }
            }
        } header: {
            Text("Loopback VPN")
        } footer: {
            Text("Turns on by itself when installing (iOS asks to allow it the first time). Its traffic never leaves this device. If installs get stuck, from any app, turn it off and on again in Settings → VPN.")
        }
    }
}

/// Installing the picked IPA, step by step
struct InstallScreen: View {
    @Environment(CompanionStore.self) private var store
    let app: CompanionStore.Picked

    @State private var steps = [StepState](pending: installSteps())
    @State private var phase = Phase.installing
    @State private var started = false

    private enum Phase {
        case installing, installed
        case failed(String)
    }

    var body: some View {
        List {
            Section {
                switch phase {
                case .installing:
                    HStack(spacing: 12) {
                        ProgressView()
                        Text("Installing \(app.name)…").font(.headline)
                    }
                case .installed:
                    Label("\(app.name) is installed", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                case .failed(let message):
                    ErrorRow(message: message)
                }
            }
            Section {
                StepListView(steps: steps)
            } footer: {
                CopyLogButton(steps: steps)
            }
        }
        .navigationTitle("Install")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            guard !started else { return }
            started = true
            await install()
        }
    }

    private func install() async {
        let recorder = StepRecorder { event in
            steps.apply(event)
            ContinuedWork.shared.update(steps)
        }
        ContinuedWork.shared.begin(title: "Installing \(app.name)", subtitle: "Starting")
        defer { ContinuedWork.shared.finish() }
        do {
            try await store.install(app, observer: recorder)
            phase = .installed
        } catch {
            phase = .failed(error.userMessage)
        }
    }
}
