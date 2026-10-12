import BackgroundTasks
import Network
import UIKit

/// Updates on their own: a background refresh now and then checks sources
/// for updates, and when there are some (and automatic updates are on), a
/// processing task makes them while the phone is charging and idle, as the
/// App Store does overnight. iOS decides when either runs, and doesn't run
/// them for an app that was force-quit or with Background App Refresh off.
enum BackgroundUpdates {
    /// The identifiers, read from Info.plist rather than built from the
    /// bundle ID, so they match however WarpSign was signed
    static let refreshId = permitted(".refresh")
    static let updatesId = permitted(".updates")

    private static func permitted(_ suffix: String) -> String? {
        let ids = Bundle.main.object(forInfoDictionaryKey: "BGTaskSchedulerPermittedIdentifiers") as? [String]
        return ids?.first { $0.hasSuffix(suffix) }
    }

    /// At launch, before it finishes
    static func register() {
        if let refreshId {
            BGTaskScheduler.shared.register(forTaskWithIdentifier: refreshId, using: nil) { task in
                guard let task = task as? BGAppRefreshTask else { return task.setTaskCompleted(success: true) }
                handleRefresh(task)
            }
        }
        if let updatesId {
            BGTaskScheduler.shared.register(forTaskWithIdentifier: updatesId, using: nil) { task in
                guard let task = task as? BGProcessingTask else { return task.setTaskCompleted(success: true) }
                handleUpdates(task)
            }
        }
    }

    /// Ask for the next check, a few hours from now
    static func scheduleRefresh() {
        guard let refreshId else { return }
        let request = BGAppRefreshTaskRequest(identifier: refreshId)
        request.earliestBeginDate = Date().addingTimeInterval(4 * 60 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    /// Ask to make the updates found, next time the phone is charging
    /// (or whenever iOS likes, with Only While Charging off)
    @MainActor
    static func scheduleUpdates() {
        guard let updatesId else { return }
        let request = BGProcessingTaskRequest(identifier: updatesId)
        request.requiresExternalPower = Updater.shared.requireCharging
        request.requiresNetworkConnectivity = true
        try? BGTaskScheduler.shared.submit(request)
    }

    private static func handleRefresh(_ task: BGAppRefreshTask) {
        scheduleRefresh()
        let once = CompleteOnce(task)
        let work = Task { @MainActor in
            let updater = Updater.shared
            await updater.check()
            if updater.automatic && !updater.automaticIds.isEmpty {
                scheduleUpdates()
            }
            once.complete(success: true)
        }
        task.expirationHandler = {
            work.cancel()
            once.complete(success: false)
        }
    }

    private static func handleUpdates(_ task: BGProcessingTask) {
        let once = CompleteOnce(task)
        let work = Task { @MainActor in
            let updater = Updater.shared
            if updater.automatic {
                if await networkAllows(wifiOnly: updater.wifiOnly) {
                    if updater.available.isEmpty { await updater.check() }
                    await updater.run(updater.automaticIds, automatic: true)
                } else {
                    // Try again later, on another network
                    scheduleUpdates()
                }
            }
            once.complete(success: true)
        }
        task.expirationHandler = {
            work.cancel()
            once.complete(success: false)
        }
    }
}

/// Whether the network suits downloading updates: connected, not in Low
/// Data Mode, and with Wi-Fi Only, not cellular (or another network iOS
/// counts as expensive, such as a hotspot)
private func networkAllows(wifiOnly: Bool) async -> Bool {
    let path = await withCheckedContinuation { (continuation: CheckedContinuation<NWPath, Never>) in
        let monitor = NWPathMonitor()
        let once = ResumeOnce()
        monitor.pathUpdateHandler = { path in
            guard once.take() else { return }
            monitor.cancel()
            continuation.resume(returning: path)
        }
        monitor.start(queue: DispatchQueue(label: "updates.network"))
    }
    guard path.status == .satisfied, !path.isConstrained else { return false }
    return !wifiOnly || !(path.isExpensive || path.usesInterfaceType(.cellular))
}

private final class ResumeOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false

    func take() -> Bool {
        lock.withLock {
            defer { done = true }
            return !done
        }
    }
}

/// Completes a background task exactly once, whether the work or iOS's
/// expiry gets there first
private final class CompleteOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var task: BGTask?

    init(_ task: BGTask) {
        self.task = task
    }

    func complete(success: Bool) {
        let task = lock.withLock {
            let task = self.task
            self.task = nil
            return task
        }
        task?.setTaskCompleted(success: success)
    }
}

/// For registering background tasks before launch finishes
final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        BackgroundUpdates.register()
        return true
    }
}
