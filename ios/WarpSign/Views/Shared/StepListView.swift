import SwiftUI
import UIKit

/// The checklist of an operation's steps. Each step expands to show its log.
struct StepListView: View {
    let steps: [StepState]

    var body: some View {
        ForEach(steps) { step in
            StepRow(title: step.title, status: step.status, log: step.log, progress: step.progress)
        }
    }
}

/// The steps of a finished operation, as kept with a signed copy
struct SavedStepListView: View {
    let steps: [SavedStep]

    var body: some View {
        ForEach(Array(steps.enumerated()), id: \.offset) { _, step in
            StepRow(title: step.title, status: step.status, log: step.log, progress: nil)
        }
    }
}

private struct StepRow: View {
    let title: String
    let status: StepState.Status
    let log: [String]
    /// 0...1 while a long step reports how far it is
    let progress: Double?
    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            if case .failed(_, let message) = status {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
            ForEach(Array(log.enumerated()), id: \.offset) { _, line in
                Text(line)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            if log.isEmpty, status == .pending {
                Text("Not run").font(.caption).foregroundStyle(.secondary)
            }
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 12) {
                    statusIcon
                        .frame(width: 22)
                    Text(title)
                    Spacer()
                    if let duration {
                        Text(duration)
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    } else if let progress {
                        Text(progress, format: .percent.precision(.fractionLength(0)))
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                if let progress, status == .running {
                    ProgressView(value: progress)
                        .padding(.leading, 34)
                }
            }
        }
        .onChange(of: status) { _, status in
            // Open a failed step straight away: that's where the answer is
            if case .failed = status { expanded = true }
        }
        .onAppear {
            if case .failed = status { expanded = true }
        }
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch status {
        case .pending:
            Image(systemName: "circle")
                .foregroundStyle(.tertiary)
        case .running:
            ProgressView()
        case .done:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .failed:
            Image(systemName: "xmark.octagon.fill")
                .foregroundStyle(.red)
        }
    }

    private var duration: String? {
        switch status {
        case .done(let ms), .failed(let ms, _):
            return ms < 1000 ? "\(ms) ms" : String(format: "%.1f s", Double(ms) / 1000)
        case .pending, .running:
            return nil
        }
    }
}

/// Copies every step's log to the clipboard
struct CopyLogButton: View {
    let transcript: String

    init(steps: [StepState]) {
        transcript = steps.transcript
    }

    init(savedSteps: [SavedStep]) {
        transcript = savedSteps.transcript
    }

    var body: some View {
        Button("Copy Log", systemImage: "doc.on.doc") {
            UIPasteboard.general.string = transcript
        }
    }
}
