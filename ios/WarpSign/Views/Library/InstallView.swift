import SwiftUI
import UIKit

/// Installs a signed IPA on this device with the chosen method, showing
/// each step
struct InstallView: View {
    @Environment(DeviceStore.self) private var device
    let ipa: URL
    let name: String
    /// Shown in iOS's install progress (web installs)
    var icon: URL?

    private enum Phase {
        case installing
        /// Web installs: the user still has to accept iOS's prompt
        case waitingForPrompt
        case installed
        case failed(String, unreachable: Bool)
    }

    @State private var phase = Phase.installing
    @State private var steps: [StepState] = []
    @State private var started = false
    @State private var method = InstallMethod.web
    @State private var server: WebInstall?

    init(ipa: URL, name: String, icon: URL? = nil) {
        self.ipa = ipa
        self.name = name
        self.icon = icon
    }

    /// Waiting for the user to accept iOS's prompt, for screenshots
    init(demoName name: String, steps: [StepState]) {
        ipa = URL(fileURLWithPath: "/dev/null")
        self.name = name
        _phase = State(initialValue: .waitingForPrompt)
        _steps = State(initialValue: steps)
        _started = State(initialValue: true)
    }

    var body: some View {
        List {
            Section { header }
            if case .waitingForPrompt = phase, let server {
                Section {
                    Button("Show the Install Prompt Again", systemImage: "arrow.clockwise") {
                        open(server)
                    }
                } footer: {
                    Text("iOS asks before installing. If you dismissed it, show it again.")
                }
            }
            if case .failed(_, true) = phase, !device.tunnel.isAvailable {
                Section {
                    Link(destination: DeviceStore.localDevVPN) {
                        Label("Get LocalDevVPN", systemImage: "network.badge.shield.half.filled")
                    }
                } footer: {
                    Text("Open LocalDevVPN and connect it, then come back and try again.")
                }
            }
            Section {
                StepListView(steps: steps)
            } header: {
                Text(method.title)
            } footer: {
                CopyLogButton(steps: steps)
            }
        }
        .navigationTitle("Install")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(isBusy)
        .toolbar {
            if case .failed = phase {
                ToolbarItem(placement: .primaryAction) {
                    Button("Try Again") { Task { await start() } }
                }
            }
            if case .waitingForPrompt = phase {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { finishWeb() }
                }
            }
        }
        .task {
            guard !started else { return }
            started = true
            await start()
        }
        .onDisappear { finishWeb() }
    }

    private var isBusy: Bool {
        if case .installing = phase { return true }
        return false
    }

    @ViewBuilder
    private var header: some View {
        switch phase {
        case .installing:
            HStack(spacing: 12) {
                ProgressView()
                VStack(alignment: .leading) {
                    Text("Installing \(name)…").font(.headline)
                    Text("Keep WarpSign open until it's sent.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        case .waitingForPrompt:
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Tap Install").font(.headline)
                    Text("iOS is asking whether to install \(name).")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } icon: {
                Image(systemName: "hand.tap.fill").foregroundStyle(.tint)
            }
        case .installed:
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text(method == .web ? "Sent to iOS" : "Installed").font(.headline)
                    Text(method == .web
                         ? "iOS is installing \(name): watch the Home Screen."
                         : "\(name) is on your Home Screen.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } icon: {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
            }
        case .failed(let message, _):
            ErrorRow(message: message)
        }
    }

    private func start() async {
        method = device.method
        // Never over the running WarpSign: iOS has to stop it mid-install,
        // and with it what's feeding the install (the VPN extension, or the
        // web server), which leaves installd stuck until a restart
        let path = ipa.path
        let bundleId = try? await runInBackground { try readIpaMetadata(ipaPath: path).bundleId }
        if let bundleId, Self.isThisApp(bundleId) {
            phase = .failed(Self.selfInstallMessage, unreachable: false)
            return
        }
        switch method {
        case .web: startWeb()
        case .pairing: await installWithPairing()
        }
    }

    private func installWithPairing() async {
        phase = .installing
        steps = [StepState](pending: installSteps())
        let recorder = StepRecorder { event in
            steps.apply(event)
            ContinuedWork.shared.update(steps)
        }
        ContinuedWork.shared.begin(title: "Installing \(name)", subtitle: "Starting")
        defer { ContinuedWork.shared.finish() }
        do {
            try await device.install(ipa, observer: recorder)
            phase = .installed
        } catch WarpsignError.DeviceUnreachable(let message) {
            phase = .failed(message, unreachable: true)
        } catch {
            phase = .failed(error.userMessage, unreachable: false)
        }
    }

    private func startWeb() {
        server?.stop()
        steps = [StepState](pending: webInstallSteps())
        let recorder = StepRecorder { event in
            steps.apply(event)
            switch event {
            case .started(.sendToIos):
                phase = .installing
            case .finished(.sendToIos, _, nil):
                phase = .installed
                // iOS may come back for a piece it missed: keep serving a bit
                Task {
                    try? await Task.sleep(for: .seconds(60))
                    finishWeb()
                }
            default:
                break
            }
        }
        do {
            let iconData = icon.flatMap { try? Data(contentsOf: $0) }
            let server = try device.startWebInstall(ipa, icon: iconData, observer: recorder)
            self.server = server
            KeepAlive.shared.start("web install")
            phase = .waitingForPrompt
            open(server)
        } catch {
            phase = .failed(error.userMessage, unreachable: false)
        }
    }

    private func open(_ server: WebInstall) {
        if let url = URL(string: server.installLink()) {
            UIApplication.shared.open(url)
        }
    }

    private func finishWeb() {
        server?.stop()
        server = nil
        KeepAlive.shared.stop("web install")
    }
}

extension InstallView {
    /// Whether `bundleId` is the WarpSign that's running
    static func isThisApp(_ bundleId: String) -> Bool {
        bundleId == Bundle.main.bundleIdentifier
    }

    static let selfInstallMessage = "This is the WarpSign you're using, and it can't install over itself: iOS has to stop it partway through, which leaves iOS unable to install anything until the device restarts. Install it with WarpSign Companion (Share Signed IPA → Companion), from a computer with `warpsign install`, or with another installer."
}

/// Install, or explain how to set installing up
struct InstallButton: View {
    @Environment(DeviceStore.self) private var device
    let ipa: URL
    let name: String
    var icon: URL?
    /// The signed copy's bundle ID, to refuse installing over this WarpSign
    var bundleId: String?

    var body: some View {
        if let bundleId, InstallView.isThisApp(bundleId) {
            Label("Can't Install Over Itself", systemImage: "nosign")
                .foregroundStyle(.secondary)
            Text(InstallView.selfInstallMessage)
                .font(.footnote)
                .foregroundStyle(.secondary)
        } else if device.isReady {
            NavigationLink {
                InstallView(ipa: ipa, name: name, icon: icon)
            } label: {
                Label("Install on This Device", systemImage: "arrow.down.app")
                    .font(.headline)
            }
        } else {
            NavigationLink {
                InstallingView()
            } label: {
                Label("Set Up Installing…", systemImage: "arrow.down.app")
            }
        }
    }
}
