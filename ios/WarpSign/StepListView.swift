import SwiftUI
import UIKit

/// The checklist of an operation's steps. Each step expands to show its log.
struct StepListView: View {
    let steps: [StepState]

    var body: some View {
        ForEach(steps) { step in
            StepRow(step: step)
        }
    }
}

private struct StepRow: View {
    let step: StepState
    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            if case .failed(_, let message) = step.status {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
            ForEach(Array(step.log.enumerated()), id: \.offset) { _, line in
                Text(line)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            if step.log.isEmpty, step.status == .pending {
                Text("Not run").font(.caption).foregroundStyle(.secondary)
            }
        } label: {
            HStack(spacing: 12) {
                statusIcon
                    .frame(width: 22)
                Text(step.title)
                Spacer()
                if let duration {
                    Text(duration)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        }
        .onChange(of: step.status) { _, status in
            // Open a failed step straight away: that's where the answer is
            if case .failed = status { expanded = true }
        }
        .onAppear {
            if case .failed = step.status { expanded = true }
        }
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch step.status {
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
        switch step.status {
        case .done(let ms), .failed(let ms, _):
            return ms < 1000 ? "\(ms) ms" : String(format: "%.1f s", Double(ms) / 1000)
        case .pending, .running:
            return nil
        }
    }
}

/// Copies every step's log to the clipboard
struct CopyLogButton: View {
    let steps: [StepState]

    var body: some View {
        Button("Copy Log", systemImage: "doc.on.doc") {
            UIPasteboard.general.string = steps.transcript
        }
    }
}
