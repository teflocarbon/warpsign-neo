import SwiftUI

// An app's page: a header card with Get and the essentials at a glance,
// then what's new, screenshots, the description and information, each in
// a card of its own on the grouped background. Sources' apps (SourceAppView) and App Store apps
// (StoreAppView) are both built from these pieces, so they look and
// behave alike.

/// The page itself: scrolls, and shows the icon and Get in the navigation
/// bar once the header has scrolled away. `action` is the Get button,
/// large for the header or compact for the bar.
struct AppPage<Content: View, Action: View>: View {
    let iconURL: String?
    var tint: Color = .accentColor
    let action: (_ large: Bool) -> Action
    let content: Content

    @State private var headerHidden = false

    init(
        iconURL: String?,
        tint: Color = .accentColor,
        @ViewBuilder action: @escaping (_ large: Bool) -> Action,
        @ViewBuilder content: () -> Content
    ) {
        self.iconURL = iconURL
        self.tint = tint
        self.action = action
        self.content = content()
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 32)
        }
        .background(Color(.systemGroupedBackground))
        .modifier(HeaderVisibility(hidden: $headerHidden))
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if headerHidden {
                ToolbarItem(placement: .principal) {
                    RemoteIconView(url: iconURL, size: 30)
                }
                ToolbarItem(placement: .primaryAction) {
                    action(false)
                }
            }
        }
        .tint(tint)
    }
}

/// The header card: icon, name, who makes it, where it's from, Get and a
/// menu, and the essentials at a glance underneath
struct AppPageHeader<Origin: View, Action: View, Menu: View>: View {
    let iconURL: String?
    let name: String
    let subtitle: String?
    let stats: [AppStat]
    let origin: Origin
    let action: Action
    let menu: Menu

    init(
        iconURL: String?,
        name: String,
        subtitle: String?,
        stats: [AppStat] = [],
        @ViewBuilder origin: () -> Origin,
        @ViewBuilder action: () -> Action,
        @ViewBuilder menu: () -> Menu = { EmptyView() }
    ) {
        self.iconURL = iconURL
        self.name = name
        self.subtitle = subtitle
        self.stats = stats
        self.origin = origin()
        self.action = action()
        self.menu = menu()
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 16) {
                RemoteIconView(url: iconURL, size: 96)
                    .contextMenu { menu }
                VStack(alignment: .leading, spacing: 3) {
                    Text(name)
                        .font(.title2.weight(.bold))
                        .lineLimit(2)
                    if let subtitle {
                        Text(subtitle)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    origin
                    Spacer(minLength: 10)
                    HStack {
                        action
                        Spacer(minLength: 8)
                        if Menu.self != EmptyView.self {
                            SwiftUI.Menu {
                                menu
                            } label: {
                                Image(systemName: "ellipsis")
                                    .font(.body.weight(.semibold))
                                    .frame(width: 34, height: 34)
                                    .background(.fill.tertiary, in: Circle())
                            }
                        }
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(minHeight: 96)
            if !stats.isEmpty {
                Divider().padding(.top, 16)
                AppStatsStrip(items: stats).padding(.top, 12)
            }
        }
        .padding(16)
        .appCard()
        .padding(.top, 8)
    }
}

extension View {
    /// A card on the grouped background, as the app page's sections
    func appCard() -> some View {
        background(
            Color(.secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: 20, style: .continuous)
        )
    }
}

/// Where an app is from, quietly, under its name: a source with its icon,
/// or the App Store
struct AppOriginLine: View {
    var iconURL: String? = nil
    var systemImage: String? = nil
    let text: String
    /// Opens somewhere (the source itself)
    var opens = false

    var body: some View {
        HStack(spacing: 5) {
            if let systemImage {
                Image(systemName: systemImage)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                RemoteIconView(url: iconURL, size: 16)
            }
            Text(text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            if opens {
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.top, 2)
    }
}

/// One of the short values in the strip under the header
struct AppStat {
    let label: String
    let value: String
    /// The unit or context, under the value
    let detail: String

    static func version(_ version: String, build: String?) -> AppStat {
        AppStat(
            label: "Version",
            value: version,
            detail: build.flatMap { $0 == version ? nil : "Build \($0)" } ?? "Latest"
        )
    }

    static func size(_ bytes: UInt64) -> AppStat {
        let count = ByteCountFormatter()
        count.countStyle = .file
        count.includesUnit = false
        let unit = ByteCountFormatter()
        unit.countStyle = .file
        unit.includesCount = false
        return AppStat(
            label: "Size",
            value: count.string(fromByteCount: Int64(clamping: bytes)),
            detail: unit.string(fromByteCount: Int64(clamping: bytes))
        )
    }

    static func requires(_ minOs: String) -> AppStat {
        let short = minOs.hasSuffix(".0") ? String(minOs.dropLast(2)) : minOs
        return AppStat(label: "Requires", value: "iOS \(short)", detail: "or later")
    }

    static func updated(_ date: Date) -> AppStat {
        AppStat(
            label: "Updated",
            value: date.formatted(.dateTime.day().month()),
            detail: date.formatted(.dateTime.year())
        )
    }
}

/// Short values with their units and context underneath, in the header
/// card
struct AppStatsStrip: View {
    let items: [AppStat]

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                if index > 0 {
                    Divider().frame(height: 40)
                }
                VStack(spacing: 3) {
                    Text(item.label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Text(item.value)
                        .font(.title3.weight(.bold))
                        .foregroundStyle(.primary.opacity(0.8))
                        .lineLimit(1)
                        .minimumScaleFactor(0.6)
                    Text(item.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                .padding(.horizontal, 4)
                .frame(maxWidth: .infinity)
            }
        }
    }
}

/// The newest version, its date and notes, and a way to the others
struct WhatsNewSection<History: View>: View {
    let version: String
    let date: Date?
    let notes: String?
    let history: History

    init(version: String, date: Date?, notes: String?, @ViewBuilder history: () -> History) {
        self.version = version
        self.date = date
        self.notes = notes
        self.history = history()
    }

    var body: some View {
        AppPageSection("What's New") {
            HStack(alignment: .firstTextBaseline) {
                Text("Version \(version)").font(.headline)
                Spacer()
                if let date {
                    Text(date, format: .dateTime.day().month().year())
                        .foregroundStyle(.secondary)
                }
            }
            if let notes, !notes.isEmpty {
                ExpandableText(notes)
            }
            history
                .font(.subheadline.weight(.semibold))
                .padding(.top, 4)
        }
    }
}

/// Screenshots, side by side, each a page wide at most
struct AppScreenshots: View {
    let urls: [String]

    var body: some View {
        if !urls.isEmpty {
            AppPageSection("Preview", card: false) {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 12) {
                        ForEach(urls, id: \.self) { url in
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
                .padding(.horizontal, -16)
                .contentMargins(.horizontal, 16, for: .scrollContent)
            }
        }
    }
}

/// The library apps that came from this page's app, to open
struct InLibrarySection: View {
    @Environment(LibraryStore.self) private var library
    let apps: [LibraryApp]
    /// Show whether each is up to date with its source
    var showsUpdates = false

    var body: some View {
        if !apps.isEmpty {
            AppPageSection("In Your Library") {
                ForEach(apps) { libraryApp in
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
                            if showsUpdates { updateStatus(libraryApp) }
                            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    @ViewBuilder
    private func updateStatus(_ libraryApp: LibraryApp) -> some View {
        if let update = Updater.shared.available[libraryApp.id] {
            Text("\(update.version.version) available").font(.subheadline).foregroundStyle(.tint)
        } else if let skipped = libraryApp.skippedVersion,
                  skipped == Updater.shared.sourceVersion(of: libraryApp)?.version.version {
            Text("Skipping \(skipped)").font(.subheadline).foregroundStyle(.secondary)
        } else {
            Text("Up to date").font(.subheadline).foregroundStyle(.green)
        }
    }
}

/// The Get button's look: a capsule, larger and bolder in the header
struct AppGetPill: View {
    let title: String
    var large = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(large ? .headline.weight(.bold) : .subheadline.weight(.bold))
                .padding(.horizontal, large ? 18 : 6)
                .padding(.vertical, large ? 2 : 0)
        }
        .buttonBorderShape(.capsule)
        .modifier(GetPillStyle(prominent: large))
    }
}

/// Filled in the header, quiet in lists
private struct GetPillStyle: ViewModifier {
    let prominent: Bool

    func body(content: Content) -> some View {
        if prominent {
            content.buttonStyle(.borderedProminent)
        } else {
            content.buttonStyle(.bordered)
        }
    }
}

/// A titled section, in a card unless it's one of its own (screenshots)
struct AppPageSection<Content: View>: View {
    let title: String
    var card = true
    let content: Content

    init(_ title: String, card: Bool = true, @ViewBuilder content: () -> Content) {
        self.title = title
        self.card = card
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.title3.weight(.bold))
                .padding(.top, 24)
                .padding(.leading, 4)
            if card {
                VStack(alignment: .leading, spacing: 10) {
                    content
                }
                // Rows end in a divider: the last one's is cut off
                .padding(.bottom, -1)
                .clipped()
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .appCard()
            } else {
                content
            }
        }
    }
}

/// A line of the Information section, to copy
struct InfoRow: View {
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

/// A line of the Information section that opens a web page
struct InfoLinkRow: View {
    let label: String
    let url: URL
    var text: String? = nil

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(label).foregroundStyle(.secondary)
                Spacer()
                Link(text ?? url.host() ?? url.absoluteString, destination: url)
            }
            .padding(.vertical, 12)
            Divider()
        }
    }
}

struct PermissionRow: View {
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
struct HeaderVisibility: ViewModifier {
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
