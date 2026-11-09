import Foundation
import Observation
import UserNotifications

/// Pairing with this device without a computer (iOS 27+): WarpSign poses
/// as a Mac on the local network (`OnDevicePairing` in warpsign-core),
/// Settings → Privacy & Security → Developer Mode offers "Pair with
/// WarpSign", and the user types the code WarpSign shows. The result is a
/// RemotePairing file, kept as if imported. StikDebug pairs the same way.
/// See docs/design/on-device-pairing.md.
@MainActor
@Observable
final class OnDevicePairer {
    enum Phase: Equatable {
        case idle
        /// Advertised: waiting for the user to tap Pair in Settings
        case waiting
        /// The code to type into Settings
        case code(String)
        case paired(name: String)
        case failed(String)
    }

    private(set) var phase: Phase = .idle
    /// The name Settings shows: "Pair with <name>"
    let hostName = "WarpSign"

    private var pairing: OnDevicePairing?
    private var service: NetService?
    @ObservationIgnored private var publishing: PublishDelegate?

    /// Device-initiated pairing came with iOS 27
    static var isSupported: Bool {
        ProcessInfo.processInfo.isOperatingSystemAtLeast(
            OperatingSystemVersion(majorVersion: 27, minorVersion: 0, patchVersion: 0)
        )
    }

    var isRunning: Bool {
        switch phase {
        case .waiting, .code: true
        default: false
        }
    }

    func start(device: DeviceStore) {
        guard !isRunning else { return }
        let pairing: OnDevicePairing
        do {
            pairing = try OnDevicePairing(name: hostName)
        } catch {
            phase = .failed(error.userMessage)
            return
        }
        self.pairing = pairing
        phase = .waiting
        Task { await Notifier.requestPermission() }
        // The user is in Settings while this waits
        KeepAlive.shared.start(Self.keepAliveReason)
        advertise(pairing)

        let observer = CodeObserver { [weak self] code in
            Task { @MainActor in
                guard let self, self.pairing === pairing else { return }
                self.phase = .code(code)
                Self.notify(code)
            }
        }
        Task {
            do {
                let paired = try await background { try pairing.accept(observer: observer) }
                // Cancelled meanwhile
                guard self.pairing === pairing else { return }
                finish()
                do {
                    try device.keepPairingFile(paired.pairingFile)
                } catch {
                    phase = .failed(error.userMessage)
                    return
                }
                phase = .paired(name: paired.name)
                await device.check()
            } catch WarpsignError.PairingCancelled {
                guard self.pairing === pairing else { return }
                finish()
                phase = .idle
            } catch {
                guard self.pairing === pairing else { return }
                finish()
                phase = .failed(error.userMessage)
            }
        }
    }

    func cancel() {
        pairing?.cancel()
        finish()
        phase = .idle
    }

    private func finish() {
        service?.stop()
        service = nil
        publishing = nil
        pairing = nil
        KeepAlive.shared.stop(Self.keepAliveReason)
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.notificationID])
    }

    /// Through mDNSResponder, which asks for Local Network access the first
    /// time (NSBonjourServices lists the type)
    private func advertise(_ pairing: OnDevicePairing) {
        let service = NetService(
            domain: "",
            type: pairing.serviceType(),
            name: pairing.serviceName(),
            port: Int32(pairing.port())
        )
        service.setTXTRecord(NetService.data(fromTXTRecord: pairing.txtRecord().mapValues { Data($0.utf8) }))
        let publishing = PublishDelegate { [weak self] code in
            Task { @MainActor in
                guard let self, self.service === service else { return }
                self.pairing?.cancel()
                self.finish()
                self.phase = .failed(
                    "WarpSign couldn't announce itself on the local network\(code.map { " (\($0))" } ?? ""). Connect to Wi-Fi, and allow Local Network access for WarpSign in Settings → Privacy & Security → Local Network, then try again."
                )
            }
        }
        service.delegate = publishing
        service.publish()
        self.service = service
        self.publishing = publishing
    }

    /// The code, for when Settings is in front
    private static func notify(_ code: String) {
        let content = UNMutableNotificationContent()
        content.title = "Pairing Code"
        content.body = "Enter \(code) in Settings to pair WarpSign with this device."
        content.sound = .default
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: notificationID, content: content, trigger: nil)
        )
    }

    private static let keepAliveReason = "pairing"
    private static let notificationID = "warpsign.pairing.code"
}

/// Reports a failed announcement (no Local Network access, usually)
private final class PublishDelegate: NSObject, NetServiceDelegate, @unchecked Sendable {
    private let failed: (Int?) -> Void

    init(_ failed: @escaping (Int?) -> Void) {
        self.failed = failed
    }

    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        failed(errorDict[NetService.errorCode]?.intValue)
    }
}

/// Hands the code from warpsign-core's thread to the pairer
private final class CodeObserver: PairingCodeObserver, @unchecked Sendable {
    private let show: (String) -> Void

    init(_ show: @escaping (String) -> Void) {
        self.show = show
    }

    func showCode(code: String) {
        show(code)
    }
}
