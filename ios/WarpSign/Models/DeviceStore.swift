import Foundation
import Observation
import UIKit

/// How WarpSign installs apps on this device
enum InstallMethod: String, CaseIterable, Identifiable {
    /// iOS's own installer, fed from a loopback server (ESign's way)
    case web
    /// installation_proxy through a loopback VPN with a pairing file
    /// (SideStore's way): WarpSign's own, or LocalDevVPN
    case pairing

    var id: Self { self }

    var title: String {
        switch self {
        case .web: "Over the Web"
        case .pairing: "With a Pairing File"
        }
    }
}

/// Installing on this device: over the web through iOS's installer, or
/// through a loopback VPN with a pairing file (made once on a computer with
/// `warpsign pair`)
@MainActor
@Observable
final class DeviceStore {
    enum Connection: Equatable {
        case unknown
        case checking
        case connected(name: String, iosVersion: String)
        /// Usually: the VPN isn't on
        case unreachable(String)
        case failed(String)
    }

    /// This device's UDID, from the pairing file
    private(set) var udid: String?
    private(set) var hasPairingFile = false
    /// RemotePairing rather than lockdown: lockdown files stopped working
    /// over the VPN in iOS 26.6
    private(set) var isRemotePairing = false
    private(set) var connection: Connection = .unknown
    var importError: String?
    /// WarpSign's own loopback VPN, turned on when it's needed
    let tunnel = LoopbackTunnel()

    static let localDevVPN = URL(string: "https://apps.apple.com/app/localdevvpn/id6755608044")!

    private static let key = "pairing"
    private static let methodKey = "installMethod"
    private static let slotKey = "installSlot"
    private static let manifestServerKey = "manifestServer"
    private static let remotePairingPortKey = "remotePairingPort"

    var method: InstallMethod {
        didSet { UserDefaults.standard.set(method.rawValue, forKey: Self.methodKey) }
    }

    /// Where web installs get their manifests (see worker/ in the repo).
    /// Empty means WarpSign's default.
    var manifestServer: String {
        didSet { UserDefaults.standard.set(manifestServer, forKey: Self.manifestServerKey) }
    }

    /// Where this device listens for RemotePairing, if not the usual port.
    /// Empty means the default.
    var remotePairingPort: String {
        didSet { UserDefaults.standard.set(remotePairingPort, forKey: Self.remotePairingPortKey) }
    }

    private var remotePairingPortOverride: UInt16? {
        UInt16(remotePairingPort.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    /// The manifest server web installs use
    var effectiveManifestServer: String {
        let custom = manifestServer.trimmingCharacters(in: .whitespacesAndNewlines)
        return custom.isEmpty ? defaultManifestServer() : custom
    }

    /// Whether installing works with the chosen method
    var isReady: Bool {
        switch method {
        case .web: effectiveManifestServer.hasPrefix("https://")
        case .pairing: hasPairingFile
        }
    }

    init() {
        method = UserDefaults.standard.string(forKey: Self.methodKey)
            .flatMap(InstallMethod.init(rawValue:)) ?? .web
        manifestServer = UserDefaults.standard.string(forKey: Self.manifestServerKey) ?? ""
        remotePairingPort = UserDefaults.standard.string(forKey: Self.remotePairingPortKey) ?? ""
        if let data = Keychain.data(for: Self.key) {
            hasPairingFile = true
            let info = try? pairingFileInfo(pairingFile: data)
            udid = info?.udid
            isRemotePairing = info?.remotePairing ?? false
        }
    }

    /// Keep a pairing file, after checking it is one. It's secret: it lets
    /// whoever has it install apps on this device.
    func importPairingFile(_ url: URL) {
        do {
            let data = try withSecurityScopedAccess(to: url) { try Data(contentsOf: url) }
            let info = try pairingFileInfo(pairingFile: data)
            try Keychain.set(data, for: Self.key)
            hasPairingFile = true
            udid = info.udid
            isRemotePairing = info.remotePairing
            connection = .unknown
            importError = nil
        } catch {
            importError = error.userMessage
        }
    }

    func removePairingFile() {
        try? Keychain.set(nil, for: Self.key)
        hasPairingFile = false
        isRemotePairing = false
        udid = nil
        connection = .unknown
    }

    /// Turn on WarpSign's own VPN if it has one. LocalDevVPN may be on
    /// instead, so failing isn't final: returns why, for if the device
    /// can't be reached.
    private func prepareTunnel() async -> String? {
        if tunnel.status == .unknown { await tunnel.refresh() }
        guard tunnel.isAvailable else { return nil }
        do {
            try await tunnel.connect()
            return nil
        } catch {
            return error.userMessage
        }
    }

    private static func unreachable(_ message: String, tunnelProblem: String?) -> String {
        guard let tunnelProblem else { return message }
        return "\(tunnelProblem) (\(message))"
    }

    /// Check the VPN and pairing file work
    func check() async {
        guard let data = Keychain.data(for: Self.key) else { return }
        connection = .checking
        let tunnelProblem = await prepareTunnel()
        let port = remotePairingPortOverride
        do {
            let device = try await background {
                try connectedDevice(pairingFile: data, remotePairingPort: port)
            }
            if udid == nil { udid = device.udid }
            connection = .connected(name: device.name, iosVersion: device.iosVersion)
        } catch WarpsignError.DeviceUnreachable(let message) {
            connection = .unreachable(Self.unreachable(message, tunnelProblem: tunnelProblem))
        } catch {
            connection = .failed(error.userMessage)
        }
    }

    /// Install a signed IPA on this device, step by step
    func install(_ ipa: URL, observer: StepRecorder) async throws {
        guard let data = Keychain.data(for: Self.key) else {
            throw WarpsignError.Device(message: "Set up installing in Settings first.")
        }
        // Copying a big app takes a while: don't let the screen lock
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = false }
        let tunnelProblem = await prepareTunnel()
        let port = remotePairingPortOverride
        do {
            try await background {
                try installIpa(
                    pairingFile: data,
                    remotePairingPort: port,
                    ipaPath: ipa.path,
                    observer: observer
                )
            }
        } catch WarpsignError.DeviceUnreachable(let message) {
            throw WarpsignError.DeviceUnreachable(
                message: Self.unreachable(message, tunnelProblem: tunnelProblem)
            )
        }
    }

    /// Start serving an IPA to iOS's installer; open the result's
    /// `installLink()` next. Uses the next manifest slot each time.
    func startWebInstall(_ ipa: URL, icon: Data?, observer: StepRecorder) throws -> WebInstall {
        let slots = webInstallSlots()
        let slot = UInt32(UserDefaults.standard.integer(forKey: Self.slotKey)) % slots
        UserDefaults.standard.set(Int((slot + 1) % slots), forKey: Self.slotKey)
        return try WebInstall.start(
            ipaPath: ipa.path,
            iconPng: icon,
            slot: slot,
            manifestServer: effectiveManifestServer,
            observer: observer
        )
    }
}
