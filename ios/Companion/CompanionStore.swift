import Foundation
import Observation
import UIKit

/// What Companion keeps: this device's pairing file, its own loopback VPN,
/// and the IPA picked to install
@MainActor
@Observable
final class CompanionStore {
    /// An IPA ready to install
    struct Picked: Sendable {
        let url: URL
        let name: String
        let version: String?
        let build: String?
        let bundleId: String
        let icon: UIImage?

        /// "0.2.0 (31)"
        var versionText: String? {
            switch (version, build) {
            case let (version?, build?) where build != version: "\(version) (\(build))"
            case let (version?, _): version
            case let (nil, build?): build
            default: nil
            }
        }

        /// This app itself, which can't install over itself
        var isCompanion: Bool { bundleId == Bundle.main.bundleIdentifier }
    }

    private(set) var hasPairingFile = false
    private(set) var udid: String?
    private(set) var picked: Picked?
    var error: String?
    /// The VPN that lets Companion reach this device's installer
    let tunnel = LoopbackTunnel()

    private static let pairingKey = "pairing"

    /// Where picked IPAs are copied, so they outlive the share sheet
    nonisolated private static var incoming: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Incoming", isDirectory: true)
    }

    init() {
        if let data = Keychain.data(for: Self.pairingKey) {
            hasPairingFile = true
            udid = (try? pairingFileInfo(pairingFile: data))?.udid
        }
    }

    /// A file opened in Companion: an IPA to install, or a pairing file
    func open(_ url: URL) {
        switch url.pathExtension.lowercased() {
        case "ipa", "zip": Task { await pick(url) }
        case "mobiledevicepairing", "plist": importPairingFile(url)
        default: error = "Companion opens .ipa files (or a .zip with one inside) and pairing files."
        }
    }

    func pick(_ url: URL) async {
        error = nil
        do {
            picked = try await runInBackground {
                let fileManager = FileManager.default
                try fileManager.createDirectory(at: Self.incoming, withIntermediateDirectories: true)
                // One at a time: the last one picked replaces the others
                for old in (try? fileManager.contentsOfDirectory(at: Self.incoming, includingPropertiesForKeys: nil)) ?? [] {
                    try? fileManager.removeItem(at: old)
                }
                var copy = Self.incoming.appendingPathComponent(url.lastPathComponent)
                try withSecurityScopedAccess(to: url) {
                    try fileManager.copyItem(at: url, to: copy)
                }
                // A zip with the IPA inside (a GitHub artifact, say): keep the IPA
                let unwrapped = Self.incoming.appendingPathComponent("unwrapped.ipa")
                if try unwrapIpa(path: copy.path, output: unwrapped.path) {
                    let ipa = copy.deletingPathExtension().appendingPathExtension("ipa")
                    try fileManager.removeItem(at: copy)
                    try fileManager.moveItem(at: unwrapped, to: ipa)
                    copy = ipa
                } else if copy.pathExtension.lowercased() != "ipa" {
                    // Already an IPA, named .zip
                    let ipa = copy.deletingPathExtension().appendingPathExtension("ipa")
                    try fileManager.moveItem(at: copy, to: ipa)
                    copy = ipa
                }
                let metadata = try readIpaMetadata(ipaPath: copy.path)
                return Picked(
                    url: copy,
                    name: metadata.displayName,
                    version: metadata.version,
                    build: metadata.build,
                    bundleId: metadata.bundleId,
                    icon: metadata.icon.flatMap(UIImage.init(data:))
                )
            }
        } catch {
            self.error = error.userMessage
        }
    }

    func importPairingFile(_ url: URL) {
        do {
            let data = try withSecurityScopedAccess(to: url) { try Data(contentsOf: url) }
            let info = try pairingFileInfo(pairingFile: data)
            try Keychain.set(data, for: Self.pairingKey)
            hasPairingFile = true
            udid = info.udid
            error = nil
        } catch {
            self.error = error.userMessage
        }
    }

    /// Install `app` on this device, step by step. If it's WarpSign, iOS
    /// closes WarpSign while it's replaced, which is fine from here.
    func install(_ app: Picked, observer: StepRecorder) async throws {
        guard !app.isCompanion else {
            throw WarpsignError.Device(message: "This is WarpSign Companion itself, which can't install over itself. Install it with WarpSign instead.")
        }
        guard let data = Keychain.data(for: Self.pairingKey) else {
            throw WarpsignError.Device(message: "Add this device's pairing file first.")
        }
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = false }
        let path = app.url.path
        try await tunnel.reachDevice(fresh: true) {
            try installIpa(pairingFile: data, remotePairingPort: nil, ipaPath: path, observer: observer)
        }
    }
}
