import Foundation
import NetworkExtension
import Observation

/// WarpSign's own loopback VPN (WarpSignTunnel.appex), which does what
/// LocalDevVPN does: installing turns it on by itself. It needs WarpSign
/// signed with Network Extensions, as signing it with WarpSign does; signed
/// some other way, LocalDevVPN is still there.
@MainActor
@Observable
final class LoopbackTunnel {
    enum Status: Equatable {
        case unknown
        /// Why it can't be used, e.g. WarpSign was signed without Network
        /// Extensions
        case unavailable(String)
        case off
        case connecting
        case on
    }

    private(set) var status: Status = .unknown

    @ObservationIgnored private var manager: NETunnelProviderManager?
    @ObservationIgnored private var observer: NSObjectProtocol?

    var isAvailable: Bool {
        if case .unavailable = status { return false }
        return true
    }

    /// The extension's bundle ID, read from the app rather than assumed, as
    /// signing may have changed it
    nonisolated static var providerBundleId: String? {
        guard let plugIns = Bundle.main.builtInPlugInsURL,
              let contents = try? FileManager.default.contentsOfDirectory(at: plugIns, includingPropertiesForKeys: nil)
        else { return nil }
        for url in contents where url.pathExtension == "appex" {
            guard let bundle = Bundle(url: url),
                  let info = bundle.infoDictionary?["NSExtension"] as? [String: Any],
                  info["NSExtensionPointIdentifier"] as? String == "com.apple.networkextension.packet-tunnel"
            else { continue }
            return bundle.bundleIdentifier
        }
        return nil
    }

    /// Find WarpSign's VPN configuration, if it has been added
    func refresh() async {
        if Demo.isOn {
            status = .on
            return
        }
        guard let providerId = Self.providerBundleId else {
            status = .unavailable("This copy of WarpSign has no VPN extension.")
            return
        }
        do {
            manager = try await NETunnelProviderManager.loadAllFromPreferences().first {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == providerId
            }
            watch()
            update()
        } catch {
            status = .unavailable(Self.explain(error))
        }
    }

    /// Turn the VPN on, adding its configuration first if needed (iOS asks
    /// to allow that once), and wait until it's up
    func connect() async throws {
        if status == .unknown { await refresh() }
        if case .unavailable(let reason) = status {
            throw WarpsignError.DeviceUnreachable(message: reason)
        }
        if status == .on { return }
        do {
            let manager = try await configured()
            status = .connecting
            try manager.connection.startVPNTunnel()
            for _ in 0..<50 where manager.connection.status != .connected {
                try await Task.sleep(for: .milliseconds(200))
            }
            update()
            if manager.connection.status != .connected {
                throw WarpsignError.DeviceUnreachable(message: "WarpSign's VPN didn't connect.")
            }
        } catch let error as WarpsignError {
            update()
            throw error
        } catch {
            let reason = Self.explain(error)
            if (error as NSError).domain == NEVPNErrorDomain,
               (error as NSError).code == NEVPNError.configurationInvalid.rawValue
                || (error as NSError).code == NEVPNError.configurationReadWriteFailed.rawValue {
                status = .unavailable(reason)
            } else {
                update()
            }
            throw WarpsignError.DeviceUnreachable(message: reason)
        }
    }

    func disconnect() {
        manager?.connection.stopVPNTunnel()
    }

    private func configured() async throws -> NETunnelProviderManager {
        if let manager, manager.isEnabled { return manager }
        guard let providerId = Self.providerBundleId else {
            throw WarpsignError.DeviceUnreachable(message: "This copy of WarpSign has no VPN extension.")
        }
        let manager = self.manager ?? NETunnelProviderManager()
        let configuration = NETunnelProviderProtocol()
        configuration.providerBundleIdentifier = providerId
        configuration.serverAddress = "This device"
        manager.protocolConfiguration = configuration
        manager.localizedDescription = "WarpSign"
        manager.isEnabled = true
        try await manager.saveToPreferences()
        // A configuration just saved can't start until it's loaded again
        try await manager.loadFromPreferences()
        self.manager = manager
        watch()
        return manager
    }

    private func watch() {
        guard observer == nil, let connection = manager?.connection else { return }
        observer = NotificationCenter.default.addObserver(
            forName: .NEVPNStatusDidChange,
            object: connection,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.update() }
        }
    }

    private func update() {
        guard let manager else {
            status = .off
            return
        }
        switch manager.connection.status {
        case .connected: status = .on
        case .connecting, .reasserting: status = .connecting
        default: status = .off
        }
    }

    private static func explain(_ error: Error) -> String {
        let error = error as NSError
        guard error.domain == NEVPNErrorDomain else { return error.localizedDescription }
        switch error.code {
        case NEVPNError.configurationReadWriteFailed.rawValue, NEVPNError.configurationInvalid.rawValue:
            return "WarpSign can't add its VPN: it wasn't signed with Network Extensions. Sign WarpSign with WarpSign to get it, or use LocalDevVPN."
        default:
            return error.localizedDescription
        }
    }
}
