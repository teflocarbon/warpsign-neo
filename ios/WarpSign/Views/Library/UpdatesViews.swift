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
                        .swipeActions {
                            Button("Skip This Version") { updater.skip(entry.app.id) }
                                .tint(.orange)
                        }
                }
            } header: {
                HStack {
                    Text("Updates")
                    Spacer()
                    if items.count > 1 {
                        Button("Update All") {
                            Task { await updater.run(items.map(\.item.appId), automatic: false) }
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
                    Task { await updater.run([app.id], automatic: false) }
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

/// Where an app's updates come from, whether it's up to date, and its
/// update settings, on its page. Apps that can't be updated say why.
struct AppUpdatesSection: View {
    @Environment(LibraryStore.self) private var library
    @Environment(SourcesStore.self) private var sources
    @Environment(DeviceStore.self) private var device
    let app: LibraryApp
    private var updater: Updater { .shared }
    @State private var linking = false
    @State private var unlinking = false

    var body: some View {
        content
            .sheet(isPresented: $linking) { LinkSourceView(app: app) }
            .confirmationDialog("Stop following \(app.origin ?? "its source")?", isPresented: $unlinking, titleVisibility: .visible) {
                Button("Stop Following", role: .destructive) {
                    library.unlink(app)
                    updater.evaluate()
                }
            } message: {
                Text("\(app.displayName) stays in the library, but isn't updated. You can link it again.")
            }
    }

    private var linkButton: some View {
        Button {
            linking = true
        } label: {
            Label("Link to a Source…", systemImage: "link")
        }
    }

    @ViewBuilder
    private var content: some View {
        switch app.updateOrigin {
        case .source(let url):
            sourceSection(url)
        case .unlinked(let name):
            Section("Updates") {
                Label {
                    Text("Downloaded from \(name), which WarpSign hasn't found in your sources. Add it again in Sources, or link it to a source, to get updates for this app.")
                } icon: {
                    Image(systemName: "exclamationmark.arrow.triangle.2.circlepath").foregroundStyle(.orange)
                }
                .font(.callout)
                linkButton
            }
        case .bundled:
            Section("Updates") {
                Label("Comes with WarpSign, and updates when WarpSign does: set it up again from the library after updating WarpSign.", systemImage: "shippingbox")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        case .imported:
            Section("Updates") {
                Label(app.link != nil
                      ? "Downloaded from a link, so there's nowhere to look for updates. Link it to a source that lists it to have WarpSign keep it up to date."
                      : "Imported from a file, so there's nowhere to look for updates. Link it to a source that lists it to have WarpSign keep it up to date.",
                      systemImage: app.link != nil ? "link" : "doc")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                linkButton
            }
        }
    }

    @ViewBuilder
    private func sourceSection(_ url: String) -> some View {
        let source = sources.sources[url]
        let sourceApp = source?.apps.first { $0.bundleId == (app.sourceAppId ?? app.bundleId) }
        Section {
            if let source, let sourceApp {
                NavigationLink {
                    SourceAppView(app: sourceApp, source: source, sourceURL: url)
                } label: {
                    LabeledContent("Source", value: source.name)
                }
            } else {
                LabeledContent("Source", value: source?.name ?? app.origin ?? URL(string: url)?.host() ?? url)
                if source != nil {
                    Text("The source no longer lists this app.")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            }

            if let item = updater.available[app.id] {
                LabeledContent("Status") {
                    Label("\(item.version.version) Available", systemImage: "arrow.down.circle.fill")
                        .foregroundStyle(.tint)
                }
                Button {
                    Task { await updater.run([app.id], automatic: false) }
                } label: {
                    Label("Update to \(item.version.version)", systemImage: "arrow.down.circle")
                }
                .disabled(updater.isUpdating)
                Button {
                    updater.skip(app.id)
                } label: {
                    Label("Skip This Version", systemImage: "forward")
                }
                .disabled(updater.isUpdating)
            } else if let match = updater.sourceVersion(of: app) {
                LabeledContent("Status") {
                    if match.version.version == app.skippedVersion {
                        Text("Skipping \(match.version.version)").foregroundStyle(.secondary)
                    } else {
                        Label("Up to Date", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                    }
                }
                LabeledContent("Newest in Source", value: versionText(match.version))
            }
            if let skipped = app.skippedVersion {
                LabeledContent {
                    Button("Don't Skip") { updater.stopSkipping(app.id) }
                } label: {
                    Label("Skipping \(skipped)", systemImage: "forward")
                }
            }
            UpdateStatusText(status: updater.status[app.id])

            Toggle("Update Automatically", isOn: Binding(
                get: { app.updatesAutomatically },
                set: { library.setAutoUpdate($0, for: app) }
            ))
            LabeledContent("Last Checked") {
                HStack(spacing: 8) {
                    if updater.isChecking {
                        ProgressView()
                    } else if let lastCheck = updater.lastCheck {
                        Text("\(lastCheck, style: .relative) ago")
                    } else {
                        Text("Never")
                    }
                    Button("Check") { Task { await updater.check() } }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        .disabled(updater.isChecking)
                }
            }
            Button("Stop Following Source", systemImage: "xmark.circle", role: .destructive) {
                unlinking = true
            }
            .disabled(updater.isUpdating)
        } header: {
            Text("Updates")
        } footer: {
            Text(footer)
        }
    }

    private func versionText(_ version: AppVersion) -> String {
        var text = version.version
        if let build = version.build, build != version.version { text += " (\(build))" }
        if let date = version.date?.prefix(10) { text += " · \(date)" }
        return text
    }

    private var footer: String {
        guard let newest = app.signedCopies.max(by: { $0.date < $1.date }) else {
            return "Sign it once: updates are signed with the options of its last signed copy."
        }
        let date = newest.date.formatted(.dateTime.day().month())
        var text = "Updates are signed as its last signed copy was (\(newest.certificate), \(date)), and replace its older signed copies. "
        switch device.method {
        case .pairing:
            text += Updater.shared.installUpdates
                ? "Then they install with your pairing file."
                : "Install Updates is off, so automatic updates wait in the library for you."
        case .web:
            text += "Installing over the web, iOS asks before each install: automatic updates wait in the library, and Update shows iOS's prompt."
        }
        return text
    }
}

/// Settings → Automatic Updates
struct UpdatesSettingsView: View {
    @Environment(DeviceStore.self) private var device
    private var updater: Updater { .shared }

    /// Over the web, iOS asks before every install
    private var overWeb: Bool { device.method == .web }

    var body: some View {
        @Bindable var updater = updater
        List {
            Section {
                Toggle("Update Apps Automatically", isOn: $updater.automatic)
                    .onChange(of: updater.automatic) { _, on in
                        if on { Task { await Notifier.requestPermission() } }
                    }
            } footer: {
                Text("Apps from your sources update in the background, the way the App Store does: downloaded, signed as they were last signed, and installed with your pairing file. You're notified when one needs you. iOS decides exactly when, and skips it if WarpSign was force-quit or Background App Refresh is off. Apps imported from files can't update.")
            }

            Section {
                Toggle("Only While Charging", isOn: $updater.requireCharging)
                Toggle("Wi-Fi Only", isOn: $updater.wifiOnly)
            } header: {
                Text("When")
            } footer: {
                Text(updater.requireCharging
                     ? "Usually overnight. Low Data Mode always holds updates back."
                     : "Whenever iOS chooses, which can be while you're using an app it updates (the app closes while it installs). Low Data Mode always holds updates back.")
            }
            .disabled(!updater.automatic)

            if overWeb {
                Section {
                    NavigationLink {
                        InstallingView()
                    } label: {
                        Label("Set Up a Pairing File…", systemImage: "cable.connector")
                    }
                } header: {
                    Text("Installing Over the Web")
                } footer: {
                    Text("iOS asks before every install over the web, so updates can't install themselves: they're downloaded and signed, then wait in the library, and tapping Update shows iOS's prompt. With a pairing file, they install on their own.")
                }
            }

            Section {
                if overWeb {
                    Toggle("Install Updates", isOn: .constant(false))
                        .disabled(true)
                } else {
                    Toggle("Install Updates", isOn: $updater.installUpdates)
                }
                Toggle("New Apps Update Automatically", isOn: $updater.newAppsUpdate)
                Toggle("Notify When Apps Update", isOn: $updater.notifySuccess)
                    .onChange(of: updater.notifySuccess) { _, on in
                        if on { Task { await Notifier.requestPermission() } }
                    }
            } header: {
                Text("Updating")
            } footer: {
                Text(updater.installsAutomatically
                     ? "New Apps applies to apps downloaded from now on; each app has its own switch on its page."
                     : "Updates are downloaded and signed, then wait in the library for you to install them. New Apps applies to apps downloaded from now on; each app has its own switch on its page.")
            }
            .disabled(!updater.automatic)

            Section {
                Toggle("Skip Betas", isOn: $updater.skipBetas)
            } footer: {
                Text("Ignores versions a source marks as pre-releases (such as 2.0-beta or 2.0b3), unless the version you have is one. Applies to updates you start too.")
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
                            } else if let note = entry.note {
                                Text(note).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Automatic Updates")
    }
}

/// Updates the user started, signed and waiting for iOS's prompt to
/// install over the web: the install screen, one at a time
struct PendingInstallSheet: ViewModifier {
    @Environment(DeviceStore.self) private var device
    @Environment(LibraryStore.self) private var library
    private var updater: Updater { .shared }

    func body(content: Content) -> some View {
        content.sheet(item: Binding(
            get: { updater.pendingInstalls.first },
            set: { if $0 == nil { next() } }
        )) { pending in
            NavigationStack {
                InstallView(ipa: pending.ipa, name: pending.name, icon: pending.icon)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button(updater.pendingInstalls.count > 1 ? "Next" : "Done") { next() }
                        }
                    }
            }
            .environment(device)
            .environment(library)
            .interactiveDismissDisabled()
        }
    }

    private func next() {
        if !updater.pendingInstalls.isEmpty { updater.pendingInstalls.removeFirst() }
    }
}

/// Where an app in the library updates from, in a word or two: so apps
/// that won't update (imported from files) stand out
struct UpdateOriginLabel: View {
    let app: LibraryApp

    var body: some View {
        Group {
            switch app.updateOrigin {
            case .source:
                if app.updatesAutomatically {
                    Label(app.origin ?? "Source", systemImage: "arrow.triangle.2.circlepath")
                        .foregroundStyle(.tint)
                } else {
                    Label("\(app.origin ?? "Source") · updates off", systemImage: "arrow.triangle.2.circlepath")
                        .foregroundStyle(.secondary)
                }
            case .unlinked(let name):
                Label("\(name) · not in your sources", systemImage: "exclamationmark.arrow.triangle.2.circlepath")
                    .foregroundStyle(.orange)
            case .bundled:
                Label("Bundled with WarpSign", systemImage: "shippingbox")
                    .foregroundStyle(.secondary)
            case .imported:
                if app.link != nil {
                    Label("From a link · no updates", systemImage: "link")
                        .foregroundStyle(.secondary)
                } else {
                    Label("Imported · no updates", systemImage: "doc")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .font(.caption)
        .labelStyle(OriginLabelStyle())
        .lineLimit(1)
    }

    /// A small icon close to its text
    private struct OriginLabelStyle: LabelStyle {
        func makeBody(configuration: Configuration) -> some View {
            HStack(spacing: 4) {
                configuration.icon.imageScale(.small)
                configuration.title
            }
        }
    }
}

/// Pick the listing in a source that an app in the library is, so it
/// updates from there without downloading it again. Listings with the
/// app's bundle ID come first; another's updates would be refused, as a
/// different app.
struct LinkSourceView: View {
    @Environment(SourcesStore.self) private var sources
    @Environment(LibraryStore.self) private var library
    @Environment(\.dismiss) private var dismiss
    let app: LibraryApp
    @State private var search = ""
    @State private var confirming: Listing?

    struct Listing: Identifiable {
        let url: String
        let source: Source
        let app: SourceApp
        var id: String { url + "\n" + app.bundleId }
    }

    private var listings: [Listing] {
        sources.urls.flatMap { url -> [Listing] in
            guard let source = sources.sources[url] else { return [] }
            return source.apps.map { Listing(url: url, source: source, app: $0) }
        }
    }

    private var matches: [Listing] { listings.filter { $0.app.bundleId == app.bundleId } }

    private var others: [Listing] {
        listings.filter { listing in
            listing.app.bundleId != app.bundleId
                && (search.isEmpty
                    || listing.app.name.localizedCaseInsensitiveContains(search)
                    || listing.source.name.localizedCaseInsensitiveContains(search)
                    || listing.app.bundleId.localizedCaseInsensitiveContains(search))
        }
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if matches.isEmpty {
                        Text("None of your sources lists \(app.bundleId). Add the source it comes from in Sources first.")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(matches) { listing in
                        Button { link(listing) } label: { row(listing) }
                    }
                } header: {
                    Text("Same App")
                } footer: {
                    Text("It keeps its signed copies and settings, and updates from the source from now on.")
                }
                if !others.isEmpty {
                    Section {
                        ForEach(others) { listing in
                            Button { confirming = listing } label: { row(listing) }
                        }
                    } header: {
                        Text("Other Apps")
                    } footer: {
                        Text("Updates must have this app's bundle ID (\(app.bundleId)), or they're refused as a different app.")
                    }
                }
            }
            .searchable(text: $search, prompt: "Search sources")
            .navigationTitle("Link to a Source")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .confirmationDialog(
                confirming.map { "Link to \($0.app.name)?" } ?? "",
                isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
                titleVisibility: .visible,
                presenting: confirming
            ) { listing in
                Button("Link Anyway") { link(listing) }
            } message: { listing in
                Text("It's listed as \(listing.app.bundleId), not \(app.bundleId): its updates are refused if they don't match this app.")
            }
        }
    }

    private func row(_ listing: Listing) -> some View {
        HStack(spacing: 12) {
            RemoteIconView(url: listing.app.iconUrl, size: 40)
            VStack(alignment: .leading, spacing: 2) {
                Text(listing.app.name).font(.headline).foregroundStyle(.primary).lineLimit(1)
                Text("\(listing.source.name) · \(listing.app.versions[0].version)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func link(_ listing: Listing) {
        library.link(
            app,
            sourceURL: listing.url,
            sourceAppId: listing.app.bundleId,
            autoUpdate: Updater.shared.newAppsUpdate,
            sourceName: listing.source.name
        )
        Updater.shared.evaluate()
        dismiss()
    }
}
