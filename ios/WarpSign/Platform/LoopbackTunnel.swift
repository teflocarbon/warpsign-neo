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
        // Screenshots (WarpSign's -demo): shown as on, nothing touched
        if ProcessInfo.processInfo.arguments.contains("-demo") {
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
    /// to allow that once), and wait until it's up. Goes by the system's
    /// live status, never a remembered one.
    func connect() async throws {
        if status == .unknown { await refresh() }
        if case .unavailable(let reason) = status {
            throw WarpsignError.DeviceUnreachable(message: reason)
        }
        if let manager, manager.connection.status == .connected {
            update()
            return
        }
        let manager: NETunnelProviderManager
        do {
            manager = try await configured()
        } catch let error as WarpsignError {
            update()
            throw error
        } catch {
            // Only a refusal to save the configuration means this copy of
            // WarpSign can't have a VPN (no Network Extensions); anything
            // else may pass, so the next try tries again
            let reason = Self.explain(error)
            if Self.isRefusal(error) {
                status = .unavailable(reason)
            } else {
                update()
            }
            throw WarpsignError.DeviceUnreachable(message: reason)
        }
        status = .connecting
        // iOS runs one VPN at a time. Just switched back from another (the
        // other app's: WarpSign's or Companion's), a tunnel often stops
        // before it's up, or won't start at all: load the configuration
        // again and try again, a few times
        var startError: Error?
        for attempt in 0..<3 {
            if attempt > 0 {
                try await Task.sleep(for: .milliseconds(600))
                try? await manager.loadFromPreferences()
                if !manager.isEnabled {
                    manager.isEnabled = true
                    try? await manager.saveToPreferences()
                    try? await manager.loadFromPreferences()
                }
            }
            do {
                try manager.connection.startVPNTunnel()
            } catch {
                startError = error
                continue
            }
            if try await waitUntilConnected(manager, seconds: attempt == 2 ? 10 : 6) {
                update()
                return
            }
        }
        update()
        let reason = await Self.lastDisconnectReason(manager) ?? startError?.localizedDescription
        throw WarpsignError.DeviceUnreachable(
            message: "\(Self.appName)'s VPN didn't connect\(reason.map { " (\($0))" } ?? ""). iOS runs one VPN at a time: if another was on (\(Self.otherAppName)'s, say), try again, or turn this one on in Settings → VPN."
        )
    }

    /// Wait for the tunnel to come up. False as soon as it's given up
    /// (back to disconnected after trying), or after `seconds`.
    private func waitUntilConnected(_ manager: NETunnelProviderManager, seconds: Double) async throws -> Bool {
        let start = Date()
        var wasConnecting = false
        while Date().timeIntervalSince(start) < seconds {
            switch manager.connection.status {
            case .connected:
                return true
            case .connecting, .reasserting:
                wasConnecting = true
            case .disconnected, .invalid:
                // Stopped after trying, or never started: give up on this go
                if wasConnecting || Date().timeIntervalSince(start) > 1.5 { return false }
            default:
                break
            }
            try await Task.sleep(for: .milliseconds(200))
        }
        return manager.connection.status == .connected
    }

    /// Why iOS last stopped the tunnel, if it says
    private static func lastDisconnectReason(_ manager: NETunnelProviderManager) async -> String? {
        await withCheckedContinuation { continuation in
            manager.connection.fetchLastDisconnectError { error in
                continuation.resume(returning: error?.localizedDescription)
            }
        }
    }

    /// This app's name, and the other one's that has a VPN too
    nonisolated static var appName: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "WarpSign"
    }

    nonisolated static var otherAppName: String {
        appName == "Companion" ? "WarpSign" : "Companion"
    }

    /// Turn the VPN off and on again: what clears a tunnel that's up but
    /// not passing anything
    func restart() async throws {
        if let manager, manager.connection.status != .disconnected {
            manager.connection.stopVPNTunnel()
            for _ in 0..<25 where manager.connection.status != .disconnected {
                try await Task.sleep(for: .milliseconds(200))
            }
        }
        update()
        try await connect()
    }

    func disconnect() {
        manager?.connection.stopVPNTunnel()
    }

    /// Run work that reaches this device through the VPN: turn it on first,
    /// and if the device can't be reached, restart it and try once more,
    /// as a tunnel can be up but stuck (after an update, say). Without this
    /// VPN (signed without Network Extensions), the work runs as it is, in
    /// case LocalDevVPN is on.
    ///
    /// `fresh` restarts a VPN that's already up first. iOS installs one app
    /// at a time, and an install whose installer vanished mid-way (killed,
    /// or its tunnel replaced) can hold every later install, from any app,
    /// for as long as its connection through the VPN looks open; taking the
    /// VPN down is what ends it.
    func reachDevice<T: Sendable>(
        fresh: Bool = false,
        _ work: @escaping @Sendable () throws -> T
    ) async throws -> T {
        if status == .unknown { await refresh() }
        guard isAvailable else { return try await runInBackground(work) }
        if fresh, let manager, manager.connection.status == .connected {
            try await restart()
        } else {
            try await connect()
        }
        do {
            return try await runInBackground(work)
        } catch WarpsignError.DeviceUnreachable {
            try await restart()
            do {
                return try await runInBackground(work)
            } catch WarpsignError.DeviceUnreachable(let message) {
                throw WarpsignError.DeviceUnreachable(
                    message: "Couldn't reach this device through the VPN, even after restarting it (\(message)). Check the pairing file is this device's."
                )
            }
        }
    }

    private func configured() async throws -> NETunnelProviderManager {
        if let manager {
            try await manager.loadFromPreferences()
            if manager.isEnabled { return manager }
        }
        guard let providerId = Self.providerBundleId else {
            throw WarpsignError.DeviceUnreachable(message: "This copy of WarpSign has no VPN extension.")
        }
        let manager = self.manager ?? NETunnelProviderManager()
        let configuration = NETunnelProviderProtocol()
        configuration.providerBundleIdentifier = providerId
        configuration.serverAddress = "This device"
        manager.protocolConfiguration = configuration
        // Each app's own name, so Settings → VPN tells them apart
        manager.localizedDescription = Self.appName
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

    /// Saving the configuration refused: no Network Extensions entitlement
    private static func isRefusal(_ error: Error) -> Bool {
        let error = error as NSError
        return error.domain == NEVPNErrorDomain
            && (error.code == NEVPNError.configurationReadWriteFailed.rawValue
                || error.code == NEVPNError.configurationInvalid.rawValue)
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
