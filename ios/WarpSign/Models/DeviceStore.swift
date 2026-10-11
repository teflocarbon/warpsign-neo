import Foundation
import Observation
import UIKit

/// Installing on this device: its pairing file (made once on a computer
/// with `warpsign pair`) and the LocalDevVPN connection, the way SideStore
/// installs
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

    init() {
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
}
