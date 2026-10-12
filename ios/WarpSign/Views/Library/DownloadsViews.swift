import SwiftUI

/// Downloads in progress, stopped or failed, at the top of the library
struct DownloadsSection: View {
    private var downloads: DownloadManager { .shared }

    var body: some View {
        if !downloads.items.isEmpty {
            Section {
                ForEach(downloads.items) { item in
                    DownloadRow(item: item)
                }
            } header: {
                Text("Downloads")
            } footer: {
                if downloads.items.contains(where: { $0.state == .downloading }) {
                    Text("Downloads carry on while WarpSign is in the background. Force-quitting it stops them; they resume where they stopped when the server allows it.")
                }
            }
        }
    }
}

/// One download: how far it is, how fast, and what can be done with it
struct DownloadRow: View {
    @Environment(LibraryStore.self) private var library
    let item: DownloadManager.Item
    private var downloads: DownloadManager { .shared }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            icon
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 6) {
                    Text(item.name).font(.headline).lineLimit(1)
                    if case .update = item.purpose {
                        Badge("Update", color: .blue)
                    }
                }
                progress
                status
            }
            Spacer(minLength: 4)
            controls
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder
    private var icon: some View {
        if case .update(let appId) = item.purpose, let image = library.icon(for: appId) {
            AppIconView(image: image, size: 44)
        } else {
            Image(systemName: "arrow.down.app.fill")
                .font(.system(size: 26))
                .foregroundStyle(.tint)
                .frame(width: 44, height: 44)
                .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
    }

    @ViewBuilder
    private var progress: some View {
        switch item.state {
        case .downloading:
            if let fraction = item.fraction {
                ProgressView(value: fraction)
            } else {
                ProgressView().progressViewStyle(.linear)
            }
        case .importing:
            ProgressView().progressViewStyle(.linear)
        case .stopped:
            if let fraction = item.fraction {
                ProgressView(value: fraction).tint(.secondary)
            }
        case .failed:
            EmptyView()
        }
    }

    @ViewBuilder
    private var status: some View {
        switch item.state {
        case .downloading:
            // Ticks so a download that's stopped moving says so
            TimelineView(.periodic(from: .now, by: 1)) { context in
                Text(Self.detail(item, now: context.date))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        case .importing:
            Text("Adding to the library…")
                .font(.caption)
                .foregroundStyle(.secondary)
        case .stopped(let reason):
            Text(item.received > 0 ? "\(reason) \(Self.sizes(item))" : reason)
                .font(.caption)
                .foregroundStyle(.orange)
        case .failed(let message):
            Text(message)
                .font(.caption)
                .foregroundStyle(.red)
        }
    }

    @ViewBuilder
    private var controls: some View {
        HStack(spacing: 14) {
            switch item.state {
            case .downloading:
                if case .library = item.purpose {
                    Button("Pause", systemImage: "pause.circle.fill") { downloads.pause(item.id) }
                }
                Button("Cancel", systemImage: "xmark.circle.fill") { downloads.cancel(item.id) }
            case .importing:
                EmptyView()
            case .stopped:
                Button("Resume", systemImage: "play.circle.fill") { downloads.resume(item.id) }
                Button("Cancel", systemImage: "xmark.circle.fill") { downloads.cancel(item.id) }
            case .failed:
                if case .library = item.purpose {
                    Button("Try Again", systemImage: "arrow.clockwise.circle.fill") { downloads.resume(item.id) }
                }
                Button("Remove", systemImage: "xmark.circle.fill") { downloads.remove(item.id) }
            }
        }
        .labelStyle(.iconOnly)
        .font(.title2)
        .symbolRenderingMode(.hierarchical)
        .buttonStyle(.borderless)
    }

    /// "12.3 MB of 137 MB · 4.2 MB/s · 25 sec left", or why it's not moving
    static func detail(_ item: DownloadManager.Item, now: Date) -> String {
        if let last = item.lastProgress, now.timeIntervalSince(last) > 8 {
            return item.received > 0
                ? "\(sizes(item)) · Waiting for the network…"
                : "Waiting for the network…"
        }
        guard item.received > 0 else { return "Starting…" }
        var parts = [sizes(item)]
        if let speed = item.bytesPerSecond, speed > 0 {
            parts.append("\(ByteCountFormatter.file(UInt64(speed)))/s")
            if let total = item.total, total > item.received {
                let seconds = Double(total - item.received) / speed
                parts.append("\(Duration.seconds(seconds.rounded()).formatted(.units(allowed: [.hours, .minutes, .seconds], width: .abbreviated, maximumUnitCount: 2))) left")
            }
        }
        return parts.joined(separator: " · ")
    }

    static func sizes(_ item: DownloadManager.Item) -> String {
        let received = ByteCountFormatter.file(UInt64(max(item.received, 0)))
        guard let total = item.total else { return received }
        return "\(received) of \(ByteCountFormatter.file(UInt64(total)))"
    }
}

/// Download from a link the user gives
struct LinkDownloadAlert: ViewModifier {
    @Environment(LibraryStore.self) private var library
    @Binding var isPresented: Bool
    @State private var link = ""

    func body(content: Content) -> some View {
        content.alert("Download from Link", isPresented: $isPresented) {
            TextField("https://example.com/app.ipa", text: $link)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            Button("Download") {
                do {
                    try DownloadManager.shared.download(link: link)
                    library.importError = nil
                } catch {
                    library.importError = error.userMessage
                }
                link = ""
            }
            Button("Cancel", role: .cancel) { link = "" }
        } message: {
            Text("A direct link to an .ipa, or a .zip with one inside, over https.")
        }
    }
}
