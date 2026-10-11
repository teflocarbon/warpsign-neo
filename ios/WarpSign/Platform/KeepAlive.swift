import AVFoundation
import UIKit

/// Keeps WarpSign running while iOS downloads an app from it, the way ESign
/// does: a silent audio loop (the audio background mode) plus a background
/// task. Without it, iOS can suspend the app mid-download and the install
/// fails.
@MainActor
final class KeepAlive {
    static let shared = KeepAlive()

    private var player: AVAudioPlayer?
    private var task: UIBackgroundTaskIdentifier = .invalid

    func start() {
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
        task = UIApplication.shared.beginBackgroundTask(withName: "Install") { [weak self] in
            Task { @MainActor in self?.endTask() }
        }
    }

    func stop() {
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
