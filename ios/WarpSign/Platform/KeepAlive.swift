import AVFoundation
import UIKit

/// Keeps WarpSign running while it has work iOS would otherwise suspend:
/// iOS downloading an app from it (a web install), or a sign or install
/// left mid-way (a backstop for when iOS takes back continued processing
/// time, and the only hold before iOS 26). The way ESign does it: a silent
/// audio loop (the audio background mode), mixed with whatever's playing,
/// plus a background task. Held only while there's work, by reason, so one
/// job finishing doesn't drop another's hold.
@MainActor
final class KeepAlive {
    static let shared = KeepAlive()

    private var reasons: Set<String> = []
    private var player: AVAudioPlayer?
    private var task: UIBackgroundTaskIdentifier = .invalid
    private var interruptions: NSObjectProtocol?

    func start(_ reason: String) {
        reasons.insert(reason)
        guard player == nil else { return }
        let session = AVAudioSession.sharedInstance()
        // Mixes with whatever's playing rather than stopping it
        try? session.setCategory(.playback, options: [.mixWithOthers])
        try? session.setActive(true)
        if let url = Bundle.main.url(forResource: "silence", withExtension: "wav"),
           let player = try? AVAudioPlayer(contentsOf: url) {
            player.numberOfLoops = -1
            player.volume = 0
            player.play()
            self.player = player
        }
        // A phone call or another app's audio stops the loop: start it again
        // once that's over, or iOS suspends the app at the next chance
        interruptions = NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification,
            object: session,
            queue: .main
        ) { [weak self] notification in
            let type = (notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt)
                .flatMap(AVAudioSession.InterruptionType.init(rawValue:))
            guard type == .ended else { return }
            MainActor.assumeIsolated {
                try? AVAudioSession.sharedInstance().setActive(true)
                self?.player?.play()
            }
        }
        task = UIApplication.shared.beginBackgroundTask(withName: "WarpSign") { [weak self] in
            Task { @MainActor in self?.endTask() }
        }
    }

    func stop(_ reason: String) {
        reasons.remove(reason)
        guard reasons.isEmpty, player != nil else { return }
        if let interruptions {
            NotificationCenter.default.removeObserver(interruptions)
        }
        interruptions = nil
        player?.stop()
        player = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        endTask()
    }

    private func endTask() {
        if task != .invalid {
            UIApplication.shared.endBackgroundTask(task)
            task = .invalid
        }
    }
}
