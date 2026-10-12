import Foundation
import Observation

/// Everything WarpSign does that takes a while, in one place: updates
/// (download, sign, install or hand to Companion), signing and installing
/// by hand. What's running, with its steps live; what's queued; and what
/// finished in the last week, with its log. Downloads stay in
/// `DownloadManager`, and Activity shows them alongside.
@MainActor
@Observable
final class ActivityCenter {
    static let shared = ActivityCenter()

    struct Job: Identifiable, Codable {
        enum Kind: String, Codable {
            case update, sign, install
        }

        enum State: Codable, Equatable {
            case queued
            case running
            /// Finished as asked
            case done
            /// Finished, but stopped short and waiting on the user
            /// (signed but not installed, say)
            case needsYou
            case failed
            case cancelled
        }

        let id: UUID
        let kind: Kind
        var title: String
        /// "21.41.5 → 21.42", or what it's doing now while running
        var detail: String?
        var appId: UUID?
        /// Started by itself (overnight), not by the user
        var automatic: Bool
        var state: State
        /// How it ended, or why it stopped
        var message: String?
        var steps: [SavedStep] = []
        let queued: Date
        var started: Date?
        var finished: Date?

        var isFinished: Bool {
            switch state {
            case .queued, .running: false
            default: true
            }
        }
    }

    /// Newest first
    private(set) var jobs: [Job] = []
    /// Running jobs' steps, as they happen
    private(set) var live: [UUID: [StepState]] = [:]
    /// Failures and jobs waiting on the user since Activity was last opened
    private(set) var unseen: Int

    var running: [Job] { jobs.filter { $0.state == .running } }
    /// In the order they'll run
    var queued: [Job] { jobs.filter { $0.state == .queued }.reversed() }
    var recent: [Job] { jobs.filter(\.isFinished) }

    private static let keep: TimeInterval = 7 * 24 * 60 * 60
    private static let unseenKey = "activity.unseen"
    private static var url: URL {
        URL.applicationSupportDirectory.appendingPathComponent("activity.json")
    }

    private init() {
        unseen = UserDefaults.standard.integer(forKey: Self.unseenKey)
        guard !Demo.isOn,
              let data = try? Data(contentsOf: Self.url),
              var saved = try? JSONDecoder().decode([Job].self, from: data)
        else { return }
        // Running or queued when WarpSign last stopped: it didn't finish
        for index in saved.indices where !saved[index].isFinished {
            saved[index].state = .failed
            saved[index].message = "Interrupted when WarpSign was closed."
            saved[index].finished = saved[index].finished ?? Date()
        }
        jobs = saved
        prune()
    }

    // MARK: Jobs

    @discardableResult
    func enqueue(_ kind: Job.Kind, title: String, detail: String? = nil, appId: UUID? = nil, automatic: Bool = false) -> UUID {
        let job = Job(
            id: UUID(), kind: kind, title: title, detail: detail, appId: appId,
            automatic: automatic, state: .queued, queued: Date()
        )
        jobs.insert(job, at: 0)
        save()
        return job.id
    }

    /// Queue and start at once (signing or installing by hand)
    @discardableResult
    func start(_ kind: Job.Kind, title: String, detail: String? = nil, appId: UUID? = nil, steps: [StepInfo] = []) -> UUID {
        let id = enqueue(kind, title: title, detail: detail, appId: appId)
        begin(id, steps: steps)
        return id
    }

    func begin(_ id: UUID, steps: [StepInfo] = []) {
        change(id) {
            $0.state = .running
            $0.started = Date()
        }
        live[id] = [StepState](pending: steps)
    }

    /// More steps for a running job: an update signs, then installs
    func addSteps(_ id: UUID, _ steps: [StepInfo]) {
        live[id, default: []].append(contentsOf: [StepState](pending: steps))
    }

    func apply(_ id: UUID, _ event: StepRecorder.Event) {
        live[id]?.apply(event)
    }

    /// What a running job is doing: "Downloading 40%", "Signing"
    func setDetail(_ id: UUID, _ detail: String) {
        guard let index = jobs.firstIndex(where: { $0.id == id }), jobs[index].detail != detail else { return }
        jobs[index].detail = detail
    }

    func finish(_ id: UUID, _ state: Job.State, message: String? = nil, detail: String? = nil) {
        change(id) {
            $0.state = state
            $0.message = message
            if let detail { $0.detail = detail }
            $0.finished = Date()
            $0.steps = (live[id] ?? []).filter { $0.status != .pending }.map(SavedStep.init)
        }
        live[id] = nil
        if state == .failed || state == .needsYou {
            unseen += 1
            UserDefaults.standard.set(unseen, forKey: Self.unseenKey)
        }
    }

    /// Take a queued job out of the queue
    func cancel(_ id: UUID) {
        guard let job = jobs.first(where: { $0.id == id }), job.state == .queued else { return }
        finish(id, .cancelled, message: "Removed from the queue.")
    }

    func isCancelled(_ id: UUID) -> Bool {
        jobs.first { $0.id == id }?.state == .cancelled
    }

    func job(_ id: UUID) -> Job? {
        jobs.first { $0.id == id }
    }

    /// Activity was opened: failures have been seen
    func markSeen() {
        unseen = 0
        UserDefaults.standard.set(0, forKey: Self.unseenKey)
    }

    func clearRecent() {
        jobs.removeAll(where: \.isFinished)
        save()
    }

    // MARK: Saving

    private func change(_ id: UUID, _ body: (inout Job) -> Void) {
        guard let index = jobs.firstIndex(where: { $0.id == id }) else { return }
        body(&jobs[index])
        save()
    }

    private func prune() {
        let cutoff = Date().addingTimeInterval(-Self.keep)
        jobs.removeAll { $0.isFinished && ($0.finished ?? $0.queued) < cutoff }
        if jobs.count > 80 { jobs = Array(jobs.prefix(80)) }
    }

    private func save() {
        guard !Demo.isOn else { return }
        prune()
        if let data = try? JSONEncoder().encode(jobs) {
            try? data.write(to: Self.url, options: .atomic)
        }
    }
}
