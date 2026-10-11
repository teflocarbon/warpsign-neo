import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// How apps get installed on this device: over the web (no setup), or
/// through LocalDevVPN with a pairing file
struct InstallingView: View {
    @Environment(DeviceStore.self) private var device
    @Environment(AccountStore.self) private var account
    @State private var importing = false
    @State private var confirmingRemove = false
    @State private var registered: Bool?
    @State private var registering = false
    @State private var registerError: String?

    static let pairingTypes: [UTType] = [
        UTType(filenameExtension: "mobiledevicepairing") ?? .data,
        .propertyList,
    ]

    var body: some View {
        @Bindable var device = device
        List {
            Section {
                Picker("Method", selection: $device.method) {
                    ForEach(InstallMethod.allCases) { method in
                        Text(method.title).tag(method)
                    }
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } footer: {
                switch device.method {
                case .web:
                    Text("No setup. WarpSign hands the app to iOS's own installer, as ESign does: iOS asks you to confirm, then installs it. Needs an internet connection to start each install.")
                case .pairing:
                    Text("WarpSign talks to this device's installer directly, as SideStore does, with clearer errors. Needs a pairing file made once on a computer, and LocalDevVPN.")
                }
            }

            if device.method == .pairing {
                pairingSections
            } else {
                manifestServerSection
            }
        }
        .navigationTitle("Installing")
        .fileImporter(isPresented: $importing, allowedContentTypes: Self.pairingTypes) { result in
            if case .success(let url) = result {
                device.importPairingFile(url)
                Task { await device.check() }
            }
        }
        .confirmationDialog("Remove the pairing file?", isPresented: $confirmingRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { device.removePairingFile() }
        } message: {
            Text("WarpSign won't be able to install through LocalDevVPN until you add one again.")
        }
    }

    private var manifestServerSection: some View {
        @Bindable var device = device
        return Section {
            TextField(
                defaultManifestServer().isEmpty ? "https://….workers.dev" : defaultManifestServer(),
                text: $device.manifestServer
            )
            .keyboardType(.URL)
            .textContentType(.URL)
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
        } header: {
            Text("Manifest Server")
        } footer: {
            Text("iOS only installs from a manifest on a trusted website, and since iOS 27 it must name the app. This server writes one for each install, naming only the app's ID, name and version; the app itself never leaves this device. Leave empty for WarpSign's own.")
        }
    }

    private var remotePairingPortSection: some View {
        @Bindable var device = device
        return Section {
            TextField("\(defaultRemotePairingPort())", text: $device.remotePairingPort)
                .keyboardType(.numberPad)
        } header: {
            Text("RemotePairing Port")
        } footer: {
            Text("Where this device listens for RemotePairing. Usually \(defaultRemotePairingPort()); leave empty unless Test Connection says it can't connect, then try the port your device advertises as _remotepairing._tcp.")
        }
    }

    @ViewBuilder
    private var pairingSections: some View {
            Section {
                if device.hasPairingFile {
                    if device.isRemotePairing {
                        Label("RemotePairing file added", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                    } else {
                        Label("Lockdown pairing file added", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        Label("iOS 26.6 and later refuse lockdown pairing files through the VPN. If Test Connection fails, make a RemotePairing file with `warpsign pair` and use that instead.", systemImage: "exclamationmark.triangle.fill")
                            .font(.footnote)
                            .foregroundStyle(.orange)
                    }
                    if let udid = device.udid {
                        LabeledContent("UDID") {
                            Text(udid).font(.caption.monospaced()).textSelection(.enabled)
                        }
                    }
                    Button("Replace Pairing File…") { importing = true }
                    Button("Remove Pairing File", role: .destructive) { confirmingRemove = true }
                } else {
                    Button("Add Pairing File…", systemImage: "doc.badge.plus") { importing = true }
                }
                if let error = device.importError {
                    ErrorRow(message: error)
                }
            } header: {
                Text("1. Pairing File")
            } footer: {
                Text("Made once on a computer: connect this device by USB and run `warpsign pair`, then get the .rppairing.plist file here (AirDrop, iCloud Drive or Files). Pairing files from idevice_pair work too. It lets apps on this device install apps, so keep it private.")
            }

            Section {
                Link(destination: DeviceStore.localDevVPN) {
                    Label("Get LocalDevVPN", systemImage: "network.badge.shield.half.filled")
                }
            } header: {
                Text("2. Loopback VPN")
            } footer: {
                Text("LocalDevVPN lets WarpSign talk to this device's installer, as SideStore does. Its traffic never leaves the device. Turn it on before installing.")
            }

            if device.isRemotePairing {
                remotePairingPortSection
            }

            Section {
                Button("Test Connection", systemImage: "bolt.horizontal") {
                    Task { await device.check() }
                }
                .disabled(!device.hasPairingFile || device.connection == .checking)
                connectionRow
            } header: {
                Text("3. Check")
            }

            if device.hasPairingFile, account.isSignedIn, let udid = device.udid {
                Section {
                    switch registered {
                    case .some(true):
                        Label("This device is registered on \(account.selectedTeam?.name ?? "your team")", systemImage: "checkmark.seal.fill")
                            .foregroundStyle(.green)
                    case .some(false):
                        Label("This device isn't registered on your team, so apps signed for it won't install.", systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        Button {
                            register(udid)
                        } label: {
                            if registering {
                                ProgressView()
                            } else {
                                Text("Register This Device")
                            }
                        }
                        .disabled(registering)
                    case .none:
                        ProgressView()
                    }
                    if let registerError {
                        ErrorRow(message: registerError)
                    }
                } header: {
                    Text("Team")
                } footer: {
                    Text("Re-sign apps after registering, so their profiles include this device.")
                }
                .task(id: udid) { await checkRegistration(udid) }
            }
    }

    @ViewBuilder
    private var connectionRow: some View {
        switch device.connection {
        case .unknown:
            EmptyView()
        case .checking:
            HStack(spacing: 12) {
                ProgressView()
                Text("Connecting…").foregroundStyle(.secondary)
            }
        case .connected(let name, let iosVersion):
            Label("Connected to \(name) (iOS \(iosVersion))", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .unreachable(let message), .failed(let message):
            ErrorRow(message: message)
        }
    }

    private func checkRegistration(_ udid: String) async {
        do {
            let devices = try await account.devices()
            registered = devices.contains { $0.udid.caseInsensitiveCompare(udid) == .orderedSame }
        } catch {
            registerError = error.userMessage
        }
    }

    private func register(_ udid: String) {
        registering = true
        registerError = nil
        Task {
            defer { registering = false }
            do {
                _ = try await account.registerDevice(name: UIDevice.current.name, udid: udid)
                registered = true
            } catch {
                registerError = error.userMessage
            }
        }
    }
}
