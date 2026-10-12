import Foundation

/// One step of an operation, as shown in the checklist
struct StepState: Identifiable {
    /// Codable so finished steps can be kept with a signed copy
    enum Status: Equatable, Codable {
        case pending
        case running
        case done(milliseconds: UInt64)
        case failed(milliseconds: UInt64, message: String)
    }

    let step: Step
    let title: String
    var status: Status = .pending
    var log: [String] = []
    /// 0...1 while a long step reports how far it is
    var progress: Double?

    var id: Step { step }

    /// A step's lines in a plain-text log
    static func transcript(title: String, status: Status, log: [String]) -> String {
        let summary: String
        switch status {
        case .pending: summary = "not run"
        case .running: summary = "running"
        case .done(let ms): summary = "ok (\(ms) ms)"
        case .failed(let ms, let message): summary = "FAILED (\(ms) ms): \(message)"
        }
        let lines = log.map { "    \($0)" }
        return (["\(title): \(summary)"] + lines).joined(separator: "\n")
    }
}

/// Receives step events from Rust on a background thread and hands them to
/// the main thread in order
final class StepRecorder: StepObserver {
    enum Event: Sendable {
        case started(Step)
        case log(Step, String)
        case progress(Step, Double)
        case finished(Step, milliseconds: UInt64, error: String?)
    }

    private let deliver: @Sendable (Event) -> Void

    init(onEvent: @escaping @MainActor @Sendable (Event) -> Void) {
        deliver = { event in
            // A serial queue keeps events in the order Rust sent them
            DispatchQueue.main.async {
                MainActor.assumeIsolated { onEvent(event) }
            }
        }
    }

    func stepStarted(step: Step) {
        deliver(.started(step))
    }

    func stepLog(step: Step, message: String) {
        deliver(.log(step, message))
    }

    func stepProgress(step: Step, fraction: Double) {
        deliver(.progress(step, fraction))
    }

    func stepFinished(step: Step, elapsedMs: UInt64, error: String?) {
        deliver(.finished(step, milliseconds: elapsedMs, error: error))
    }
}

extension Array where Element == StepState {
    /// The steps an operation will run, all pending
    init(pending steps: [StepInfo]) {
        self = steps.map { StepState(step: $0.step, title: $0.title) }
    }

    mutating func apply(_ event: StepRecorder.Event) {
        switch event {
        case .started(let step):
            // Started again (signing restarts after signing in again)
            update(step) {
                $0.status = .running
                $0.log = []
            }
        case .log(let step, let message):
            update(step) { $0.log.append(message) }
        case .progress(let step, let fraction):
            update(step) { $0.progress = fraction }
        case .finished(let step, let milliseconds, let error):
            update(step) {
                $0.progress = nil
                if let error {
                    $0.status = .failed(milliseconds: milliseconds, message: error)
                } else {
                    $0.status = .done(milliseconds: milliseconds)
                }
            }
        }
    }

    private mutating func update(_ step: Step, _ change: (inout StepState) -> Void) {
        if let index = firstIndex(where: { $0.step == step }) {
            change(&self[index])
        }
    }

    /// Plain-text log of every step, for copying into a bug report
    var transcript: String {
        map { StepState.transcript(title: $0.title, status: $0.status, log: $0.log) }
            .joined(separator: "\n")
    }
}
