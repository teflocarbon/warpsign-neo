import SwiftUI

/// An app in a source, laid out like the App Store and TestFlight: header
/// with Get, the essentials at a glance, what's new, screenshots, the
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
    @State private var headerHidden = false

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
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if isOnlyApp {
                    VStack(alignment: .leading, spacing: 8) {
                        Label {
                            Text("This is the only app in \(source.name), so it opens straight here.")
                        } icon: {
                            Image(systemName: "lightbulb.fill").foregroundStyle(.yellow)
                        }
                        if let sourceURL {
                            NavigationLink {
                                SourceDetailView(url: sourceURL, showsSource: true)
                            } label: {
                                Text(source.news.isEmpty ? "About This Source" : "About This Source and Its News")
                                    .font(.subheadline.weight(.semibold))
                            }
                        }
                    }
                    .font(.callout)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .padding(.bottom, 16)
                }
                header
                stats
                if let download {
                    DownloadRow(item: download)
                        .padding(12)
                        .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .padding(.top, 12)
                }
                if !inLibrary.isEmpty { libraryRows }
                whatsNew
                screenshots
                if let description = app.description { about(description) }
                information
                permissions
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 32)
        }
        .modifier(HeaderVisibility(hidden: $headerHidden))
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if headerHidden {
                ToolbarItem(placement: .principal) {
                    RemoteIconView(url: app.iconUrl, size: 30)
                }
                ToolbarItem(placement: .primaryAction) {
                    DownloadButton(version: latest, app: app, source: source)
                }
            }
        }
        .tint(tint)
        .task { names = await account.cachedEntitlementNames() }
    }

    // MARK: Header

    private var header: some View {
        HStack(alignment: .top, spacing: 16) {
            RemoteIconView(url: app.iconUrl, size: 112)
                .contextMenu { SourceAppMenu(app: app, source: source) }
            VStack(alignment: .leading, spacing: 4) {
                Text(app.name)
                    .font(.title2.weight(.bold))
                    .lineLimit(2)
                if let subtitle = app.subtitle ?? app.developerName {
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 8)
                DownloadButton(version: latest, app: app, source: source, large: true)
            }
            Spacer(minLength: 0)
        }
        .frame(minHeight: 112)
        .padding(.top, 8)
    }

    /// Version, size, requirement and date, as the App Store's strip
    private var stats: some View {
        let size: (String, String)? = latest.size.map { ("Size", ByteCountFormatter.file($0)) }
        let requires: (String, String)? = latest.minOs.map { ("Requires", "iOS \($0)") }
        let updated: (String, String)? = latest.date.flatMap(SourceDate.parse)
            .map { ("Updated", $0.formatted(.dateTime.day().month())) }
        let candidates: [(String, String)?] = [("Version", latest.version), size, requires, updated]
        let items = candidates.compactMap { $0 }
        return VStack(spacing: 0) {
            Divider().padding(.top, 20)
            HStack(spacing: 0) {
                ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                    if index > 0 {
                        Divider().frame(height: 36)
                    }
                    VStack(spacing: 4) {
                        Text(item.0.uppercased())
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text(item.1)
                            .font(.title3.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .minimumScaleFactor(0.6)
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .padding(.vertical, 12)
            Divider()
        }
    }

    private var libraryRows: some View {
        SourceSection("In Your Library") {
            ForEach(inLibrary) { libraryApp in
                NavigationLink {
                    AppDetailView(appId: libraryApp.id)
                } label: {
                    HStack(spacing: 12) {
                        AppIconView(image: library.icon(for: libraryApp.id), size: 40)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(libraryApp.displayName).font(.headline).foregroundStyle(.primary)
                            Text(libraryApp.versionText).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if let update = Updater.shared.available[libraryApp.id] {
                            Text("\(update.version.version) available").font(.subheadline).foregroundStyle(.tint)
                        } else if let skipped = libraryApp.skippedVersion,
                                  skipped == Updater.shared.sourceVersion(of: libraryApp)?.version.version {
                            Text("Skipping \(skipped)").font(.subheadline).foregroundStyle(.secondary)
                        } else {
                            Text("Up to date").font(.subheadline).foregroundStyle(.green)
                        }
                        Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                    }
                }
                .buttonStyle(.plain)
            }
        }
    }

    // MARK: Sections

    private var whatsNew: some View {
        SourceSection("What's New") {
            HStack(alignment: .firstTextBaseline) {
                Text("Version \(latest.version)").font(.headline)
                Spacer()
                if let date = latest.date.flatMap(SourceDate.parse) {
                    Text(date, format: .dateTime.day().month().year())
                        .foregroundStyle(.secondary)
                }
            }
            if let notes = latest.notes {
                ExpandableText(notes)
            }
            if app.versions.count > 1 {
                NavigationLink {
                    SourceVersionsView(app: app, source: source)
                } label: {
                    Text("Version History")
                        .font(.subheadline.weight(.semibold))
                }
                .padding(.top, 4)
            }
        }
    }

    @ViewBuilder
    private var screenshots: some View {
        if !app.screenshotUrls.isEmpty {
            SourceSection("Preview") {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 12) {
                        ForEach(app.screenshotUrls, id: \.self) { url in
                            AsyncImage(url: URL(string: url)) { image in
                                image.resizable().scaledToFit()
                            } placeholder: {
                                Color.gray.opacity(0.15).aspectRatio(9.0 / 19.5, contentMode: .fit)
                            }
                            .frame(height: 420)
                            .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
                            .overlay {
                                RoundedRectangle(cornerRadius: 22, style: .continuous)
                                    .strokeBorder(.quaternary, lineWidth: 0.5)
                            }
                        }
                    }
                    .scrollTargetLayout()
                }
                .scrollTargetBehavior(.viewAligned)
                .padding(.horizontal, -20)
                .contentMargins(.horizontal, 20, for: .scrollContent)
            }
        }
    }

    private func about(_ description: String) -> some View {
        SourceSection("Description") {
            ExpandableText(description)
        }
    }

    private var information: some View {
        SourceSection("Information") {
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
                    HStack {
                        Text("Website").foregroundStyle(.secondary)
                        Spacer()
                        Link(url.host() ?? website, destination: url)
                    }
                    .padding(.vertical, 12)
                }
            }
        }
    }

    @ViewBuilder
    private var permissions: some View {
        if !app.entitlements.isEmpty || !app.privacy.isEmpty {
            SourceSection("Permissions") {
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

// MARK: - Pieces

/// A titled section, as the App Store's
private struct SourceSection<Content: View>: View {
    let title: String
    let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.title2.weight(.bold))
                .padding(.top, 24)
            content
        }
    }
}

private struct InfoRow: View {
    let label: String
    let value: String
    var monospaced = false

    init(_ label: String, _ value: String, monospaced: Bool = false) {
        self.label = label
        self.value = value
        self.monospaced = monospaced
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 16) {
                Text(label).foregroundStyle(.secondary)
                Spacer(minLength: 8)
                Text(value)
                    .font(monospaced ? .body.monospaced() : .body)
                    .multilineTextAlignment(.trailing)
            }
            .padding(.vertical, 12)
            .contentShape(Rectangle())
            .contextMenu { CopyMenu(title: label, value: value) }
            Divider()
        }
    }
}

private struct PermissionRow: View {
    let symbol: String
    let title: String
    let detail: String
    var monospacedDetail = false

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.body)
                .foregroundStyle(.tint)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(detail)
                    .font(monospacedDetail ? .caption.monospaced() : .callout)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
        .contentShape(Rectangle())
        .contextMenu {
            CopyMenu(title: "Name", value: title)
            CopyMenu(title: monospacedDetail ? "Key" : "Reason", value: detail)
        }
    }
}

/// Long text, cut to a few lines until "more" is tapped. "more" shows
/// only when the text really is cut: it's measured at full height
/// against the height it's cut to.
struct ExpandableText: View {
    let text: String
    var lines = 4
    var secondary = true
    @State private var expanded = false
    @State private var shownHeight: CGFloat = 0
    @State private var fullHeight: CGFloat = 0

    init(_ text: String, lines: Int = 4, secondary: Bool = true) {
        self.text = text
        self.lines = lines
        self.secondary = secondary
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text)
                .foregroundStyle(secondary ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                .lineLimit(expanded ? nil : lines)
                .frame(maxWidth: .infinity, alignment: .leading)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shownHeight = $0 }
                .background(alignment: .top) {
                    // The whole text, unseen, at the same width
                    Text(text)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .hidden()
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fullHeight = $0 }
                }
            if !expanded && fullHeight > shownHeight + 1 {
                Button("more") {
                    withAnimation(.easeInOut(duration: 0.2)) { expanded = true }
                }
                .font(.subheadline.weight(.semibold))
                .buttonStyle(.borderless)
            }
        }
    }
}

/// Shows the app in the navigation bar once its header scrolls away
/// (iOS 18 can tell; on iOS 17 the bar stays plain)
private struct HeaderVisibility: ViewModifier {
    @Binding var hidden: Bool

    func body(content: Content) -> some View {
        if #available(iOS 18.0, *) {
            content.onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y + geometry.contentInsets.top > 150
            } action: { _, hidden in
                withAnimation(.easeInOut(duration: 0.2)) { self.hidden = hidden }
            }
        } else {
            content
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
        Button(action: action) {
            Text(title)
                .font(large ? .headline.weight(.bold) : .subheadline.weight(.bold))
                .padding(.horizontal, large ? 18 : 6)
                .padding(.vertical, large ? 2 : 0)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.capsule)
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
