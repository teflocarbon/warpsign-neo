import SwiftUI

/// Apps whose source has a newer version, at the top of the library
struct UpdatesSection: View {
    @Environment(LibraryStore.self) private var library
    private var updater: Updater { .shared }

    var body: some View {
        let items = updater.available.values
            .compactMap { item in library.app(item.appId).map { (app: $0, item: item) } }
            .sorted { $0.app.displayName.localizedCaseInsensitiveCompare($1.app.displayName) == .orderedAscending }
        if !items.isEmpty {
            Section {
                ForEach(items, id: \.item.id) { entry in
                    UpdateRow(app: entry.app, item: entry.item)
                }
            } header: {
                HStack {
                    Text("Updates")
                    Spacer()
                    if items.count > 1 {
                        Button("Update All") {
                            Task { await updater.run(items.map(\.item.appId), showCard: true, notify: false) }
                        }
                        .disabled(updater.isUpdating)
                        .font(.subheadline.weight(.semibold))
                        .textCase(nil)
                    }
                }
            }
        }
    }
}

private struct UpdateRow: View {
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    let item: Updater.Available
    private var updater: Updater { .shared }

    var body: some View {
        HStack(spacing: 12) {
            AppIconView(image: library.icon(for: app.id), size: 44)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.displayName).font(.headline).lineLimit(1)
                Text("\(app.versionText) → \(item.version.version) · \(item.sourceName)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                UpdateStatusText(status: updater.status[app.id])
            }
            Spacer()
            if updater.status[app.id].map(Self.isBusy) != true {
                Button("Update") {
                    Task { await updater.run([app.id], showCard: true, notify: false) }
                }
                .buttonStyle(.bordered)
                .disabled(updater.isUpdating)
            } else {
                ProgressView()
            }
        }
    }

    private static func isBusy(_ status: Updater.Status) -> Bool {
        switch status {
        case .waiting, .downloading, .signing, .installing: true
        case .done, .failed: false
        }
    }
}

/// Where an update is up to, or how it ended
struct UpdateStatusText: View {
    let status: Updater.Status?

    var body: some View {
        switch status {
        case .none:
            EmptyView()
        case .waiting:
            Text("Waiting…").font(.caption).foregroundStyle(.secondary)
        case .downloading(let fraction):
            Text("Downloading \(Int(fraction * 100))%").font(.caption).foregroundStyle(.secondary)
        case .signing:
            Text("Signing…").font(.caption).foregroundStyle(.secondary)
        case .installing:
            Text("Installing…").font(.caption).foregroundStyle(.secondary)
        case .done(let message):
            Text(message).font(.caption).foregroundStyle(.green)
        case .failed(let message):
            Text(message).font(.caption).foregroundStyle(.red)
        }
    }
}

/// An app's source and updates, on its page
struct AppUpdatesSection: View {
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    private var updater: Updater { .shared }

    var body: some View {
        if app.sourceURL != nil {
            Section {
                if let item = updater.available[app.id] {
                    Button {
                        Task { await updater.run([app.id], showCard: true, notify: false) }
                    } label: {
                        Label("Update to \(item.version.version)", systemImage: "arrow.down.circle")
                    }
                    .disabled(updater.isUpdating)
                }
                UpdateStatusText(status: updater.status[app.id])
                Toggle("Update Automatically", isOn: Binding(
                    get: { app.updatesAutomatically },
                    set: { library.setAutoUpdate($0, for: app) }
                ))
            } header: {
                Text("From \(app.origin ?? "a source")")
            } footer: {
                if app.signedCopies.isEmpty {
                    Text("Updates are downloaded and signed with the options this app was last signed with, then installed. Sign it once first.")
                } else {
                    Text("Updates are downloaded and signed with the options this app was last signed with, then installed.")
                }
            }
        }
    }
}

/// Settings → Automatic Updates
struct UpdatesSettingsView: View {
    private var updater: Updater { .shared }

    var body: some View {
        @Bindable var updater = updater
        List {
            Section {
                Toggle("Update Apps Automatically", isOn: $updater.automatic)
                    .onChange(of: updater.automatic) { _, on in
                        if on { Task { await Notifier.requestPermission() } }
                    }
            } footer: {
                Text("Apps from your sources update overnight while this iPhone is charging, the way the App Store does: downloaded, signed as they were last signed, and installed with your pairing file. You're only notified when one needs you. iOS decides exactly when, and skips it if WarpSign was force-quit or Background App Refresh is off.")
            }

            Section {
                Button {
                    Task { await updater.check() }
                } label: {
                    HStack {
                        Label("Check Now", systemImage: "arrow.clockwise")
                        Spacer()
                        if updater.isChecking { ProgressView() }
                    }
                }
                .disabled(updater.isChecking)
                if let lastCheck = updater.lastCheck {
                    LabeledContent("Last Checked") {
                        Text("\(lastCheck, style: .relative) ago")
                    }
                }
                LabeledContent("Updates Available", value: "\(updater.available.count)")
            }

            if !updater.history.isEmpty {
                Section("Recent Updates") {
                    ForEach(updater.history.prefix(20)) { entry in
                        VStack(alignment: .leading, spacing: 2) {
                            HStack {
                                Text(entry.appName).font(.headline)
                                Spacer()
                                Text(entry.date, format: .dateTime.day().month().hour().minute())
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Text("\(entry.from) → \(entry.to)").font(.caption).foregroundStyle(.secondary)
                            if let problem = entry.problem {
                                Text(problem).font(.caption).foregroundStyle(.orange)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Automatic Updates")
    }
}
