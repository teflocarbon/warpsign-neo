import SwiftUI
import UIKit

/// Activity, as a sheet from the library
struct ActivitySheet: View {
    @Environment(\.dismiss) private var dismiss
    /// Open an app's page (closing the sheet)
    var open: ((UUID) -> Void)?

    var body: some View {
        NavigationStack {
            ActivityList(open: open.map { open in
                { id in
                    dismiss()
                    open(id)
                }
            })
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

/// What's running, what's next, downloads, and what finished lately
struct ActivityList: View {
    @Environment(LibraryStore.self) private var library
    var open: ((UUID) -> Void)?
    private var activity: ActivityCenter { .shared }
    private var downloads: DownloadManager { .shared }

    /// Downloads into the library (an update's shows on its job)
    private var libraryDownloads: [DownloadManager.Item] {
        downloads.items.filter { if case .library = $0.purpose { true } else { false } }
    }

    var body: some View {
        List {
            if !activity.running.isEmpty {
                Section("Now") {
                    ForEach(activity.running) { job in
                        RunningJobView(job: job)
                    }
                }
            }
            if !libraryDownloads.isEmpty {
                Section("Downloads") {
                    ForEach(libraryDownloads) { item in
                        DownloadRow(item: item)
                            .contextMenu { DownloadMenu(item: item) }
                    }
                }
            }
            if !activity.queued.isEmpty {
                Section("Up Next") {
                    ForEach(activity.queued) { job in
                        JobRow(job: job)
                            .swipeActions {
                                Button("Remove", role: .destructive) { activity.cancel(job.id) }
                            }
                            .contextMenu {
                                Button("Remove from Queue", systemImage: "xmark", role: .destructive) {
                                    activity.cancel(job.id)
                                }
                            }
                    }
                }
            }
            if !activity.recent.isEmpty {
                Section {
                    ForEach(activity.recent) { job in
                        NavigationLink {
                            JobDetailView(jobId: job.id, open: open)
                        } label: {
                            JobRow(job: job)
                        }
                        .contextMenu { jobMenu(job) }
                    }
                } header: {
                    Text("Recent")
                } footer: {
                    Text("The last week, with each one's log.")
                }
            }
        }
        .overlay {
            if activity.jobs.isEmpty && downloads.items.isEmpty {
                ContentUnavailableView(
                    "Nothing Yet",
                    systemImage: "clock.arrow.circlepath",
                    description: Text("Updates, signing, installs and downloads show here while they run, and for a week after.")
                )
            }
        }
        .navigationTitle("Activity")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if !activity.recent.isEmpty {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Clear") { activity.clearRecent() }
                }
            }
        }
        .onAppear { activity.markSeen() }
    }

    @ViewBuilder
    private func jobMenu(_ job: ActivityCenter.Job) -> some View {
        if let appId = job.appId, library.app(appId) != nil, let open {
            Button("Open App", systemImage: "arrow.up.right.square") { open(appId) }
        }
        if let appId = job.appId, job.kind == .update, Updater.shared.canUpdate(appId) {
            Button("Try Again", systemImage: "arrow.clockwise") {
                Task { await Updater.shared.run([appId], automatic: false) }
            }
        }
        if !job.steps.isEmpty {
            Button("Copy Log", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = job.steps.transcript
            }
        }
        if let message = job.message {
            Button("Copy Message", systemImage: "text.bubble") {
                UIPasteboard.general.string = message
            }
        }
    }
}

/// A running job: what it is, what it's doing, and its steps
private struct RunningJobView: View {
    let job: ActivityCenter.Job

    /// An update's download, while it's downloading
    private var download: DownloadManager.Item? {
        guard job.kind == .update, let appId = job.appId else { return nil }
        return DownloadManager.shared.items.first { $0.purpose == .update(appId: appId) }
    }

    var body: some View {
        JobRow(job: job)
        if let download {
            DownloadRow(item: download)
        }
        let steps = ActivityCenter.shared.live[job.id] ?? []
        if !steps.isEmpty {
            StepListView(steps: steps)
        }
    }
}

/// One job, in a line or two
struct JobRow: View {
    @Environment(LibraryStore.self) private var library
    let job: ActivityCenter.Job

    var body: some View {
        HStack(spacing: 12) {
            icon
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(job.title).font(.headline).lineLimit(1)
                    if job.automatic {
                        Image(systemName: "moon.fill")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("Automatic")
                    }
                }
                if let detail = job.detail {
                    Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                if let message = job.message, job.isFinished {
                    Text(message)
                        .font(.caption)
                        .foregroundStyle(messageColor)
                        .lineLimit(2)
                }
            }
            Spacer(minLength: 4)
            VStack(alignment: .trailing, spacing: 4) {
                stateBadge
                if let when = job.finished ?? job.started {
                    Text(when, format: .relative(presentation: .named))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
        }
    }

    @ViewBuilder
    private var icon: some View {
        if let appId = job.appId, let image = library.icon(for: appId) {
            AppIconView(image: image, size: 40)
        } else {
            Image(systemName: job.kind == .sign ? "signature" : job.kind == .install ? "arrow.down.app" : "arrow.triangle.2.circlepath")
                .font(.title3)
                .foregroundStyle(.tint)
                .frame(width: 40, height: 40)
                .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
        }
    }

    @ViewBuilder
    private var stateBadge: some View {
        switch job.state {
        case .queued: Badge("Queued", color: .secondary)
        case .running: ProgressView().controlSize(.small)
        case .done: Badge("Done", color: .green)
        case .needsYou: Badge("Needs You", color: .orange)
        case .failed: Badge("Failed", color: .red)
        case .cancelled: Badge("Cancelled", color: .secondary)
        }
    }

    private var messageColor: Color {
        switch job.state {
        case .failed: .red
        case .needsYou: .orange
        default: .secondary
        }
    }
}

/// A finished job: how it went, its steps and log, and what next
private struct JobDetailView: View {
    @Environment(LibraryStore.self) private var library
    let jobId: UUID
    var open: ((UUID) -> Void)?
    private var activity: ActivityCenter { .shared }

    var body: some View {
        if let job = activity.job(jobId) {
            List {
                Section {
                    JobRow(job: job)
                    if let message = job.message {
                        Text(message).font(.callout).textSelection(.enabled)
                    }
                }
                Section {
                    if let appId = job.appId, library.app(appId) != nil, let open {
                        Button("Open App", systemImage: "arrow.up.right.square") { open(appId) }
                    }
                    if let appId = job.appId, job.kind == .update, Updater.shared.canUpdate(appId) {
                        Button("Try Again", systemImage: "arrow.clockwise") {
                            Task { await Updater.shared.run([appId], automatic: false) }
                        }
                        .disabled(Updater.shared.isUpdating)
                    }
                }
                if !job.steps.isEmpty {
                    Section {
                        SavedStepListView(steps: job.steps)
                    } header: {
                        Text("Steps")
                    } footer: {
                        CopyLogButton(savedSteps: job.steps)
                    }
                }
                Section {
                    LabeledContent("Queued") { Text(job.queued, format: .dateTime.day().month().hour().minute()) }
                    if let started = job.started {
                        LabeledContent("Started") { Text(started, format: .dateTime.hour().minute().second()) }
                    }
                    if let finished = job.finished {
                        LabeledContent("Finished") { Text(finished, format: .dateTime.hour().minute().second()) }
                    }
                    LabeledContent("Started By", value: job.automatic ? "Automatic updates" : "You")
                }
            }
            .navigationTitle(job.title)
            .navigationBarTitleDisplayMode(.inline)
        } else {
            ContentUnavailableView("Cleared", systemImage: "clock.arrow.circlepath")
        }
    }
}

/// A slim line at the top of the library while something's going, or
/// when something needs a look: opens Activity
struct ActivityStrip: View {
    let open: () -> Void
    private var activity: ActivityCenter { .shared }
    private var downloads: DownloadManager { .shared }

    /// Whether it has anything to say (so the library shows no empty row)
    @MainActor
    static var isShown: Bool {
        !ActivityCenter.shared.running.isEmpty
            || ActivityCenter.shared.unseen > 0
            || !DownloadManager.shared.items.isEmpty
    }

    private var activeDownloads: Int {
        downloads.items.filter { $0.state == .downloading || $0.state == .importing }.count
    }

    var body: some View {
        let running = activity.running
        let others = running.count - 1 + activeDownloads + activity.queued.count
        if let job = running.first {
            Button(action: open) {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(job.title).font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                        Text([job.detail, others > 0 ? "and \(others) more" : nil].compactMap { $0 }.joined(separator: " · "))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                }
            }
        } else if activeDownloads > 0 {
            Button(action: open) {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Downloading \(activeDownloads) app\(activeDownloads == 1 ? "" : "s")")
                        .font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                    Spacer()
                    Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                }
            }
        } else if activity.unseen > 0 || downloads.items.contains(where: { $0.state != .downloading && $0.state != .importing }) {
            Button(action: open) {
                HStack(spacing: 10) {
                    Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange)
                    Text(activity.unseen > 0
                         ? "\(activity.unseen) thing\(activity.unseen == 1 ? "" : "s") need\(activity.unseen == 1 ? "s" : "") a look"
                         : "A download stopped")
                        .font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                    Spacer()
                    Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                }
            }
        }
    }
}

/// The library's Activity button: a dot while something's running, red
/// when something needs a look
struct ActivityButton: View {
    let open: () -> Void
    private var activity: ActivityCenter { .shared }
    private var busy: Bool {
        !activity.running.isEmpty || DownloadManager.shared.activeCount > 0
    }

    var body: some View {
        Button(action: open) {
            Image(systemName: "clock.arrow.circlepath")
                .overlay(alignment: .topTrailing) {
                    if activity.unseen > 0 || busy {
                        Circle()
                            .fill(activity.unseen > 0 ? Color.red : Color.accentColor)
                            .frame(width: 8, height: 8)
                            .offset(x: 3, y: -3)
                    }
                }
        }
        .accessibilityLabel("Activity")
    }
}
