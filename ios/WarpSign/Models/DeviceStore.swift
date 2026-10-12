import Foundation
import Observation
import UIKit

/// How WarpSign installs apps on this device. A pairing file is the way to
/// go: silent installs, so updates install themselves, and WarpSign can
/// see what's installed. Over the web is for when there's no computer to
/// make one with.
enum InstallMethod: String, CaseIterable, Identifiable {
    /// installation_proxy through a loopback VPN with a pairing file
    /// (SideStore's way): WarpSign's own, or LocalDevVPN
    case pairing
    /// iOS's own installer, fed from a loopback server (ESign's way)
    case web

    var id: Self { self }

    var title: String {
        switch self {
        case .pairing: "With a Pairing File"
        case .web: "Over the Web"
        }
    }

    /// What it's for, in the Installing picker
    var subtitle: String {
        switch self {
        case .pairing: "Recommended. Installs without asking, so updates install themselves, and WarpSign sees what's installed. A computer once."
        case .web: "Without a computer. iOS asks before every install."
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
    /// What's installed on this device, by bundle ID: nil until it's been
    /// asked, which needs a pairing file. Every app shows, however it was
    /// installed.
    private(set) var installed: [String: InstalledApp]?
    private(set) var installedChecked: Date?
    private(set) var isCheckingInstalled = false
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
        let pairing = Keychain.data(for: Self.key)
        // Not chosen yet: a pairing file if there is one
        method = UserDefaults.standard.string(forKey: Self.methodKey)
            .flatMap(InstallMethod.init(rawValue:)) ?? (pairing == nil ? .web : .pairing)
        manifestServer = UserDefaults.standard.string(forKey: Self.manifestServerKey) ?? ""
        remotePairingPort = UserDefaults.standard.string(forKey: Self.remotePairingPortKey) ?? ""
        if let pairing {
            hasPairingFile = true
            let info = try? pairingFileInfo(pairingFile: pairing)
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
            // The better way: use it
            method = .pairing
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

    /// Check the VPN and pairing file work
    func check() async {
        guard let data = Keychain.data(for: Self.key) else { return }
        connection = .checking
        let port = remotePairingPortOverride
        do {
            let device = try await tunnel.reachDevice {
                try connectedDevice(pairingFile: data, remotePairingPort: port)
            }
            if udid == nil { udid = device.udid }
            connection = .connected(name: device.name, iosVersion: device.iosVersion)
        } catch WarpsignError.DeviceUnreachable(let message) {
            connection = .unreachable(message)
        } catch {
            connection = .failed(error.userMessage)
        }
    }

    /// Ask the device what's installed. With `connecting`, turns the VPN
    /// on to ask; without, asks only if it's already on (so opening the
    /// library doesn't start the VPN). Keeps the last answer on failure.
    func refreshInstalled(connecting: Bool = true) async {
        guard !Demo.isOn, hasPairingFile, !isCheckingInstalled,
              let data = Keychain.data(for: Self.key)
        else { return }
        if !connecting {
            if tunnel.status == .unknown { await tunnel.refresh() }
            guard tunnel.status == .on else { return }
        }
        isCheckingInstalled = true
        defer { isCheckingInstalled = false }
        let port = remotePairingPortOverride
        do {
            let apps = try await tunnel.reachDevice {
                try installedApps(pairingFile: data, remotePairingPort: port)
            }
            installed = Dictionary(apps.map { ($0.bundleId, $0) }, uniquingKeysWith: { first, _ in first })
            installedChecked = Date()
        } catch {
            // The last answer stands
        }
    }

    /// The same, unless it was asked in the last couple of minutes
    func refreshInstalledIfDue(connecting: Bool = false) async {
        if let installedChecked, Date().timeIntervalSince(installedChecked) < 120 { return }
        await refreshInstalled(connecting: connecting)
    }

    /// A library app as installed on this device: any of its signed
    /// copies' bundle IDs (newest first), or its own. Nil if it isn't, or
    /// if that isn't known (`installed` is nil).
    func installedApp(_ app: LibraryApp) -> InstalledApp? {
        guard let installed else { return nil }
        let ids = app.signedCopies.sorted { $0.date > $1.date }.map(\.bundleId) + [app.bundleId]
        return ids.lazy.compactMap { installed[$0] }.first
    }

    /// Whether a library app that's been signed isn't on this device (it
    /// was deleted, say): false when that isn't known
    func isMissing(_ app: LibraryApp) -> Bool {
        installed != nil && !app.signedCopies.isEmpty && installedApp(app) == nil
    }

    /// Install a signed IPA on this device, step by step
    func install(_ ipa: URL, observer: StepRecorder) async throws {
        guard let data = Keychain.data(for: Self.key) else {
            throw WarpsignError.Device(message: "Set up installing in Settings first.")
        }
        // Copying a big app takes a while: don't let the screen lock
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = false }
        let port = remotePairingPortOverride
        let path = ipa.path
        try await tunnel.reachDevice(fresh: true) {
            try installIpa(
                pairingFile: data,
                remotePairingPort: port,
                ipaPath: path,
                observer: observer
            )
        }
        // What's installed changed: the VPN is up, so ask now
        Task { await refreshInstalled(connecting: false) }
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
