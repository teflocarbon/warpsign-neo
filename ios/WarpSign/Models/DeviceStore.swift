import Foundation
import Observation
import UIKit

/// How WarpSign installs apps on this device
enum InstallMethod: String, CaseIterable, Identifiable {
    /// iOS's own installer, fed from a loopback server (ESign's way)
    case web
    /// installation_proxy through LocalDevVPN with a pairing file (SideStore's way)
    case pairing

    var id: Self { self }

    var title: String {
        switch self {
        case .web: "Over the Web"
        case .pairing: "Through LocalDevVPN"
        }
    }
}

/// Installing on this device: over the web through iOS's installer, or
/// through LocalDevVPN with a pairing file (made once on a computer with
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
    private(set) var connection: Connection = .unknown
    var importError: String?

    static let localDevVPN = URL(string: "https://apps.apple.com/app/localdevvpn/id6755608044")!

    private static let key = "pairing"
    private static let methodKey = "installMethod"
    private static let slotKey = "installSlot"
    private static let manifestServerKey = "manifestServer"

    var method: InstallMethod {
        didSet { UserDefaults.standard.set(method.rawValue, forKey: Self.methodKey) }
    }

    /// Where web installs get their manifests (see worker/ in the repo).
    /// Empty means WarpSign's default.
    var manifestServer: String {
        didSet { UserDefaults.standard.set(manifestServer, forKey: Self.manifestServerKey) }
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
        if let data = Keychain.data(for: Self.key) {
            hasPairingFile = true
            udid = (try? pairingFileInfo(pairingFile: data))?.udid
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
            connection = .unknown
            importError = nil
        } catch {
            importError = error.userMessage
        }
    }

    func removePairingFile() {
        try? Keychain.set(nil, for: Self.key)
        hasPairingFile = false
        udid = nil
        connection = .unknown
    }

    /// Check the VPN and pairing file work
    func check() async {
        guard let data = Keychain.data(for: Self.key) else { return }
        connection = .checking
        do {
            let device = try await background { try connectedDevice(pairingFile: data) }
            if udid == nil { udid = device.udid }
            connection = .connected(name: device.name, iosVersion: device.iosVersion)
        } catch WarpsignError.DeviceUnreachable(let message) {
            connection = .unreachable(message)
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
        try await background {
            try installIpa(pairingFile: data, ipaPath: ipa.path, observer: observer)
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
