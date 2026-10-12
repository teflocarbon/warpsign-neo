import BackgroundTasks
import Foundation
import os

/// iOS 26's continued processing: a sign or install the user started keeps
/// running after they leave WarpSign, with the system's own progress card in
/// the Dynamic Island and on the Lock Screen. Earlier iOS just doesn't get
/// the card.
///
/// One request at a time, for the job on screen. The task is always handed
/// back as successful, however the job went: the job's outcome is shown by
/// WarpSign, and a task handed back as failed (or never handed back) makes
/// iOS show its own failure card that the app can't clear.
@MainActor
final class ContinuedWork {
    static let shared = ContinuedWork()

    /// The request's identifier, "<bundle ID>.continued.job", made from
    /// Info.plist's permitted pattern rather than the bundle ID, so the two
    /// match however WarpSign was signed
    nonisolated static let identifier: String? = {
        let permitted = Bundle.main.object(forInfoDictionaryKey: "BGTaskSchedulerPermittedIdentifiers") as? [String]
        guard let pattern = permitted?.first(where: { $0.hasSuffix(".continued.*") }) else { return nil }
        return String(pattern.dropLast()) + "job"
    }()

    /// The running task, touched from the scheduler's queue as well as here
    nonisolated private static let grant = Grant()
    private static let log = Logger(subsystem: "WarpSign", category: "continued")

    private var registered = false
    private var refused = false
    private var wanted = false
    private var title = ""
    private var subtitle = ""
    private var fraction = 0.0

    /// Whether leaving WarpSign mid-job is fine
    var isAvailable: Bool {
        guard #available(iOS 26.0, *) else { return false }
        return Self.identifier != nil && !refused
    }

    /// Ask to keep going in the background, with the card titled `title`
    func begin(title: String, subtitle: String) {
        self.title = title
        self.subtitle = subtitle
        fraction = 0
        wanted = true
        // Underneath: iOS can take the time back early, and before iOS 26
        // there's nothing else
        KeepAlive.shared.start("job")
        guard #available(iOS 26.0, *), let identifier = Self.identifier, !refused else { return }
        if !registered {
            registered = true
            // Once per process: registering twice is fatal
            let accepted = BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: nil) { task in
                Self.started(task)
            }
            if !accepted {
                refused = true
                return
            }
        }
        let request = BGContinuedProcessingTaskRequest(identifier: identifier, title: title, subtitle: subtitle)
        // Now or not at all: a card that turns up after the job is over
        // would be for nothing
        request.strategy = .fail
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            Self.log.error("continued processing refused: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// Show how far the job is
    func update(_ steps: [StepState]) {
        guard wanted else { return }
        if let running = steps.runningTitle { subtitle = running }
        fraction = steps.fractionDone
        Self.grant.show(title: title, subtitle: subtitle, fraction: fraction)
    }

    /// Show a step of a job made of several (updates), and how far it is
    func update(subtitle: String, fraction: Double) {
        guard wanted else { return }
        self.subtitle = subtitle
        self.fraction = fraction
        Self.grant.show(title: title, subtitle: subtitle, fraction: fraction)
    }

    /// The job is over, however it went
    func finish() {
        wanted = false
        KeepAlive.shared.stop("job")
        Self.grant.complete()
        if #available(iOS 26.0, *), let identifier = Self.identifier {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: identifier)
        }
    }

    /// On the scheduler's queue. The expiration handler is set here, before
    /// anything waits for the main actor, so iOS taking the time back is
    /// always answered.
    nonisolated private static func started(_ task: BGTask) {
        guard #available(iOS 26.0, *), let task = task as? BGContinuedProcessingTask else {
            task.setTaskCompleted(success: true)
            return
        }
        grant.hold(task)
        task.expirationHandler = { grant.complete() }
        Task { @MainActor in shared.adopt() }
    }

    private func adopt() {
        if wanted {
            Self.grant.show(title: title, subtitle: subtitle, fraction: fraction)
        } else {
            Self.grant.complete()
        }
    }
}

/// The task iOS started, completed exactly once from whichever thread gets
/// there first
private final class Grant: @unchecked Sendable {
    private let lock = NSLock()
    private var task: BGTask?

    func hold(_ task: BGTask) {
        lock.withLock { self.task = task }
    }

    func show(title: String, subtitle: String, fraction: Double) {
        lock.withLock {
            guard #available(iOS 26.0, *), let task = task as? BGContinuedProcessingTask else { return }
            task.updateTitle(title, subtitle: subtitle)
            task.progress.totalUnitCount = 100
            task.progress.completedUnitCount = Int64((min(max(fraction, 0), 1) * 100).rounded())
        }
    }

    func complete() {
        let task = lock.withLock {
            let task = self.task
            self.task = nil
            return task
        }
        task?.setTaskCompleted(success: true)
    }
}

extension Array where Element == StepState {
    /// How far through all the steps, counting the running one's progress
    var fractionDone: Double {
        guard !isEmpty else { return 0 }
        let done = reduce(0.0) { sum, step -> Double in
            switch step.status {
            case .done, .failed: return sum + 1
            case .running: return sum + (step.progress ?? 0)
            case .pending: return sum
            }
        }
        return done / Double(count)
    }

    var runningTitle: String? {
        first { $0.status == .running }?.title
    }
}
