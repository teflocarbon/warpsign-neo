import Foundation
import UIKit

/// Hands a signed WarpSign to WarpSign Companion to install, as WarpSign
/// can't install over itself: serves it on loopback (nothing but this
/// device can reach it) and opens Companion with where to fetch it.
/// WarpSign keeps serving in the background until Companion has it.
@MainActor
enum CompanionHandoff {
    static let scheme = "warpsign-companion"
    private static var share: LoopbackShare?
    private static var stopping: Task<Void, Never>?

    /// `tunnel` (WarpSign's VPN) is turned off first: iOS runs one VPN at
    /// a time, and Companion's starts cleanly once WarpSign's is down
    /// rather than while it's going
    static func send(_ ipa: URL, name: String, tunnel: LoopbackTunnel?) async throws {
        stop()
        if let tunnel, tunnel.status == .on || tunnel.status == .connecting {
            tunnel.disconnect()
            for _ in 0..<15 where tunnel.status == .on || tunnel.status == .connecting {
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
        let slot = UInt32.random(in: 0..<webInstallSlots())
        let recorder = StepRecorder { event in
            // Sent: give Companion a moment, then stop serving
            if case .finished(.sendToIos, _, nil) = event {
                stop(after: 30)
            }
        }
        let share = try LoopbackShare.start(ipaPath: ipa.path, slot: slot, observer: recorder)
        self.share = share
        KeepAlive.shared.start("companion")

        var components = URLComponents()
        components.scheme = scheme
        components.host = "install"
        components.queryItems = [
            URLQueryItem(name: "url", value: share.url()),
            URLQueryItem(name: "name", value: name),
        ]
        guard let link = components.url, await UIApplication.shared.open(link) else {
            stop()
            throw WarpsignError.Device(
                message: "WarpSign Companion isn't installed: set it up from the library (it comes with WarpSign), then try again."
            )
        }
        // Never serve for long
        stop(after: 300)
    }

    static func stop() {
        stopping?.cancel()
        stopping = nil
        share?.stop()
        share = nil
        KeepAlive.shared.stop("companion")
    }

    private static func stop(after seconds: Double) {
        stopping?.cancel()
        stopping = Task {
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            stop()
        }
    }
}
