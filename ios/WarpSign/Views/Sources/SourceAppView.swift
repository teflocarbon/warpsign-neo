import SwiftUI

/// An app in a source, on the shared app page (AppPage.swift): header with
/// Get, the essentials at a glance, what's new, screenshots, the
/// description, information, and the permissions the source lists, by
/// name
struct SourceAppView: View {
    @Environment(SourcesStore.self) private var sources
    @Environment(LibraryStore.self) private var library
    @Environment(AccountStore.self) private var account
    let app: SourceApp
    let source: Source
    /// The source's address, to show the source itself
    var sourceURL: String? = nil
    /// Opened straight from the source, as its only app
    var isOnlyApp = false

    @State private var names = EntitlementNames()

    private var latest: AppVersion { app.versions[0] }
    private var tint: Color { Color(hex: app.tintColor) ?? Color(hex: source.tintColor) ?? .accentColor }

    /// Its newest version's download, while there is one
    private var download: DownloadManager.Item? {
        DownloadManager.shared.items.first { $0.url.absoluteString == latest.downloadUrl }
    }

    /// Library apps downloaded from (or linked to) this listing
    private var inLibrary: [LibraryApp] {
        library.apps.filter { libraryApp in
            guard let url = libraryApp.sourceURL else { return false }
            return sources.sources[url]?.name == source.name
                && (libraryApp.sourceAppId ?? libraryApp.bundleId) == app.bundleId
        }
    }

    var body: some View {
        AppPage(iconURL: app.iconUrl, tint: tint) { large in
            DownloadButton(version: latest, app: app, source: source, large: large)
        } content: {
            AppPageHeader(
                iconURL: app.iconUrl, name: app.name, subtitle: app.subtitle ?? app.developerName, stats: statItems
            ) {
                if let sourceURL {
                    NavigationLink {
                        SourceDetailView(url: sourceURL, showsSource: true)
                    } label: {
                        sourceLine
                    }
                    .buttonStyle(.plain)
                } else {
                    sourceLine
                }
            } action: {
                DownloadButton(version: latest, app: app, source: source, large: true)
            } menu: {
                SourceAppMenu(app: app, source: source)
            }
            if let download {
                DownloadRow(item: download)
                    .padding(12)
                    .appCard()
                    .padding(.top, 12)
            }
            InLibrarySection(apps: inLibrary, showsUpdates: true)
            WhatsNewSection(
                version: latest.version,
                date: latest.date.flatMap(SourceDate.parse),
                notes: latest.notes
            ) {
                if app.versions.count > 1 {
                    NavigationLink("Version History") {
                        SourceVersionsView(app: app, source: source)
                    }
                }
            }
            AppScreenshots(urls: app.screenshotUrls)
            if let description = app.description {
                AppPageSection("Description") {
                    ExpandableText(description)
                }
            }
            information
            permissions
        }
        .task { names = await account.cachedEntitlementNames() }
    }

    /// The source (and that it's its only app, when it opened straight
    /// here), to open the source itself
    private var sourceLine: some View {
        AppOriginLine(
            iconURL: source.iconUrl,
            text: isOnlyApp ? "\(source.name) · only app" : source.name,
            opens: sourceURL != nil
        )
    }

    private var statItems: [AppStat] {
        var items = [AppStat.version(latest.version, build: latest.build)]
        if let size = latest.size { items.append(.size(size)) }
        if let minOs = latest.minOs { items.append(.requires(minOs)) }
        if let date = latest.date.flatMap(SourceDate.parse) { items.append(.updated(date)) }
        return items
    }

    // MARK: Sections

    private var information: some View {
        AppPageSection("Information") {
            VStack(spacing: 0) {
                InfoRow("Bundle ID", app.bundleId, monospaced: true)
                InfoRow("Version", latest.build.map { $0 == latest.version ? latest.version : "\(latest.version) (\($0))" } ?? latest.version)
                if let category = app.category {
                    InfoRow("Category", category.capitalized)
                }
                if let date = latest.date.flatMap(SourceDate.parse) {
                    InfoRow("Updated", date.formatted(.dateTime.day().month().year()))
                }
                if let size = latest.size {
                    InfoRow("Size", ByteCountFormatter.file(size))
                }
                if let minOs = latest.minOs {
                    InfoRow("Requires", "iOS \(minOs) or later")
                }
                if let developer = app.developerName {
                    InfoRow("Developer", developer)
                }
                if let sourceURL {
                    NavigationLink {
                        SourceDetailView(url: sourceURL, showsSource: true)
                    } label: {
                        HStack {
                            Text("Source").foregroundStyle(.secondary)
                            Spacer()
                            Text(source.name).foregroundStyle(.tint)
                            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                        }
                        .padding(.vertical, 12)
                    }
                    .buttonStyle(.plain)
                    Divider()
                } else {
                    InfoRow("Source", source.name)
                }
                if let website = source.website, let url = URL(string: website) {
                    InfoLinkRow(label: "Website", url: url)
                }
            }
        }
    }

    @ViewBuilder
    private var permissions: some View {
        if !app.entitlements.isEmpty || !app.privacy.isEmpty {
            AppPageSection("Permissions") {
                if !app.entitlements.isEmpty {
                    Text("Capabilities")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    VStack(spacing: 0) {
                        ForEach(sortedEntitlements, id: \.self) { key in
                            PermissionRow(
                                symbol: names[key]?.symbol ?? "checkmark.seal",
                                title: names[key]?.name ?? EntitlementTitle.guess(key),
                                detail: key,
                                monospacedDetail: true
                            )
                        }
                    }
                }
                if !app.privacy.isEmpty {
                    Text("Privacy")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.top, app.entitlements.isEmpty ? 0 : 12)
                    VStack(spacing: 0) {
                        ForEach(app.privacy, id: \.key) { permission in
                            let info = PrivacyPermissionInfo(permission.key)
                            PermissionRow(
                                symbol: info.symbol,
                                title: info.title,
                                detail: permission.reason ?? permission.key,
                                monospacedDetail: permission.reason == nil
                            )
                        }
                    }
                }
                Text("As the source lists them. WarpSign signs with what the app itself asks for, and tells you what it couldn't grant.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.top, 8)
            }
        }
    }

    private var sortedEntitlements: [String] {
        app.entitlements.sorted {
            (names[$0]?.name ?? EntitlementTitle.guess($0))
                .localizedCaseInsensitiveCompare(names[$1]?.name ?? EntitlementTitle.guess($1)) == .orderedAscending
        }
    }
}

/// Every version a source lists, in its order (newest first), each cut
/// to a few lines until opened, and each to download
struct SourceVersionsView: View {
    let app: SourceApp
    let source: Source

    var body: some View {
        List {
            Section {
                ForEach(Array(app.versions.enumerated()), id: \.offset) { index, version in
                    row(index, version)
                        .contextMenu { SourceVersionMenu(version: version) }
                }
            } footer: {
                Text("Getting an older version asks what happens to updates, so it isn't updated straight back: skip the newer version, or stop it, or every app from \(source.name), updating by itself.")
            }
        }
        .listStyle(.plain)
        .navigationTitle("Version History")
        .navigationBarTitleDisplayMode(.large)
        .tint(Color(hex: app.tintColor) ?? Color(hex: source.tintColor))
    }

    private func row(_ index: Int, _ version: AppVersion) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text("Version \(version.version)")
                    .foregroundStyle(.secondary)
                if index == 0 {
                    Badge("Latest", color: .green)
                }
                Spacer()
                if let date = version.date.flatMap(SourceDate.parse) {
                    Text(date, format: .relative(presentation: .named))
                        .foregroundStyle(.secondary)
                }
            }
            if let notes = version.notes {
                ExpandableText(notes, lines: 3, secondary: false)
            }
            HStack {
                Text([
                    version.build.flatMap { $0 == version.version ? nil : "Build \($0)" },
                    version.size.map { ByteCountFormatter.file($0) },
                    version.minOs.map { "iOS \($0)+" },
                ].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                Spacer()
                DownloadButton(version: version, app: app, source: source)
            }
        }
        .padding(.vertical, 6)
    }
}

/// Downloads a version into the library, showing progress
struct DownloadButton: View {
    @Environment(SourcesStore.self) private var sources
    let version: AppVersion
    let app: SourceApp
    let source: Source
    /// The header's, larger and filled
    var large = false
    @State private var choosing = false

    /// Not the source's latest: getting it asks what happens to updates
    private var isOlder: Bool { version.downloadUrl != app.versions.first?.downloadUrl }

    /// This version's download, while there is one
    private var item: DownloadManager.Item? {
        DownloadManager.shared.items.first { $0.url.absoluteString == version.downloadUrl }
    }

    var body: some View {
        switch item?.state {
        case .downloading:
            if let fraction = item?.fraction {
                ProgressView(value: fraction)
                    .progressViewStyle(.circular)
                    .frame(width: 28, height: 28)
            } else {
                ProgressView().frame(width: 28, height: 28)
            }
        case .importing:
            ProgressView().frame(width: 28, height: 28)
        case .stopped, .failed:
            pill("Retry") {
                if let item { DownloadManager.shared.resume(item.id) }
            }
            .tint(.orange)
        case nil:
            pill("Get") {
                if isOlder {
                    choosing = true
                } else {
                    sources.download(version, of: app, from: source)
                }
            }
            .confirmationDialog(
                "Get \(app.name) \(version.version)?",
                isPresented: $choosing,
                titleVisibility: .visible
            ) {
                let latest = app.versions.first?.version ?? "the latest"
                Button("Skip \(latest)") {
                    sources.download(version, of: app, from: source, older: .skipNewer)
                }
                Button("Don't Update This App") {
                    sources.download(version, of: app, from: source, older: .noAutomaticUpdates)
                }
                Button("Don't Update Apps from \(source.name)") {
                    sources.download(version, of: app, from: source, older: .noAutomaticUpdatesFromSource)
                }
            } message: {
                Text("It's older than \(app.versions.first?.version ?? "the latest"), so it would be updated straight back. Skip \(app.versions.first?.version ?? "it") (later versions still update it), or stop it, or every app from \(source.name), updating by itself. You can still update by hand, and change this on its page.")
            }
        }
    }

    private func pill(_ title: String, action: @escaping () -> Void) -> some View {
        AppGetPill(title: title, large: large, action: action)
    }
}

/// Dates as sources give them: ISO 8601, with or without a time
enum SourceDate {
    static func parse(_ text: String) -> Date? {
        let withTime = ISO8601DateFormatter()
        if let date = withTime.date(from: text) { return date }
        withTime.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withTime.date(from: text) { return date }
        let day = ISO8601DateFormatter()
        day.formatOptions = [.withFullDate]
        return day.date(from: String(text.prefix(10)))
    }
}

/// A readable name for an entitlement key the team's capabilities don't
/// name (not signed in yet, or not a capability):
/// "com.apple.developer.associated-domains" → "Associated Domains"
enum EntitlementTitle {
    static func guess(_ key: String) -> String {
        let last = key.split(separator: ".").last.map(String.init) ?? key
        return last
            .split(whereSeparator: { $0 == "-" || $0 == "_" })
            .map { $0.prefix(1).uppercased() + $0.dropFirst() }
            .joined(separator: " ")
    }
}

/// Privacy permissions by their Info.plist key: what they're called in
/// Settings, and a symbol
struct PrivacyPermissionInfo {
    let title: String
    let symbol: String

    init(_ key: String) {
        let known: [String: (String, String)] = [
            "NSCameraUsageDescription": ("Camera", "camera"),
            "NSMicrophoneUsageDescription": ("Microphone", "mic"),
            "NSPhotoLibraryUsageDescription": ("Photos", "photo.on.rectangle"),
            "NSPhotoLibraryAddUsageDescription": ("Add to Photos", "photo.badge.plus"),
            "NSContactsUsageDescription": ("Contacts", "person.crop.circle"),
            "NSCalendarsUsageDescription": ("Calendars", "calendar"),
            "NSCalendarsFullAccessUsageDescription": ("Calendars", "calendar"),
            "NSCalendarsWriteOnlyAccessUsageDescription": ("Add to Calendars", "calendar.badge.plus"),
            "NSRemindersUsageDescription": ("Reminders", "checklist"),
            "NSRemindersFullAccessUsageDescription": ("Reminders", "checklist"),
            "NSLocationUsageDescription": ("Location", "location"),
            "NSLocationWhenInUseUsageDescription": ("Location While Using", "location"),
            "NSLocationAlwaysUsageDescription": ("Location Always", "location.fill"),
            "NSLocationAlwaysAndWhenInUseUsageDescription": ("Location Always", "location.fill"),
            "NSFaceIDUsageDescription": ("Face ID", "faceid"),
            "NSSpeechRecognitionUsageDescription": ("Speech Recognition", "waveform"),
            "NSUserTrackingUsageDescription": ("Tracking", "hand.raised"),
            "NSBluetoothAlwaysUsageDescription": ("Bluetooth", "antenna.radiowaves.left.and.right"),
            "NSBluetoothPeripheralUsageDescription": ("Bluetooth", "antenna.radiowaves.left.and.right"),
            "NSLocalNetworkUsageDescription": ("Local Network", "network"),
            "NSMotionUsageDescription": ("Motion & Fitness", "figure.walk"),
            "NSHealthShareUsageDescription": ("Health", "heart"),
            "NSHealthUpdateUsageDescription": ("Health (Write)", "heart"),
            "NSHomeKitUsageDescription": ("Home", "house"),
            "NSAppleMusicUsageDescription": ("Media & Apple Music", "music.note"),
            "NSSiriUsageDescription": ("Siri", "mic.badge.plus"),
            "NFCReaderUsageDescription": ("NFC", "wave.3.right"),
            "NSFocusStatusUsageDescription": ("Focus Status", "moon"),
            "NSNearbyInteractionUsageDescription": ("Nearby Interactions", "dot.radiowaves.left.and.right"),
        ]
        if let entry = known[key] {
            title = entry.0
            symbol = entry.1
        } else {
            // "NSFooBarUsageDescription" → "Foo Bar"
            let core = key
                .replacingOccurrences(of: "UsageDescription", with: "")
                .replacingOccurrences(of: "NS", with: "", options: .anchored)
            title = core.replacingOccurrences(of: "([a-z])([A-Z])", with: "$1 $2", options: .regularExpression)
            symbol = "lock.shield"
        }
    }
}
