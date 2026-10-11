import SwiftUI

/// Installs a signed IPA on this device, showing each step
struct InstallView: View {
    @Environment(DeviceStore.self) private var device
    let ipa: URL
    let name: String

    private enum Phase {
        case installing
        case installed
        case failed(String, unreachable: Bool)
    }

    @State private var phase = Phase.installing
    @State private var steps = [StepState](pending: installSteps())
    @State private var started = false

    var body: some View {
        List {
            Section { header }
            if case .failed(_, true) = phase {
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
                Text("Steps")
            } footer: {
                CopyLogButton(steps: steps)
            }
        }
        .navigationTitle("Install")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(isInstalling)
        .toolbar {
            if case .failed = phase {
                ToolbarItem(placement: .primaryAction) {
                    Button("Try Again") {
                        Task { await install() }
                    }
                }
            }
        }
        .task {
            guard !started else { return }
            started = true
            await install()
        }
    }

    private var isInstalling: Bool {
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
                    Text("Keep WarpSign open until it's copied.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        case .installed:
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Installed").font(.headline)
                    Text("\(name) is on your Home Screen.")
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

    private func install() async {
        phase = .installing
        steps = [StepState](pending: installSteps())
        let recorder = StepRecorder { event in
            steps.apply(event)
        }
        do {
            try await device.install(ipa, observer: recorder)
            phase = .installed
        } catch WarpsignError.DeviceUnreachable(let message) {
            phase = .failed(message, unreachable: true)
        } catch {
            phase = .failed(error.userMessage, unreachable: false)
        }
    }
}

/// Install, or explain how to set installing up
struct InstallButton: View {
    @Environment(DeviceStore.self) private var device
    let ipa: URL
    let name: String

    var body: some View {
        if device.hasPairingFile {
            NavigationLink {
                InstallView(ipa: ipa, name: name)
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
