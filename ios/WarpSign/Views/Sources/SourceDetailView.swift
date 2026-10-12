import SwiftUI
import UIKit

/// A source: its header, description and news, then its apps. A source
/// with one app opens straight to it, unless `showsSource`.
struct SourceDetailView: View {
    @Environment(SourcesStore.self) private var sources
    let url: String
    /// Show the source itself, even with only one app ("About This Source")
    var showsSource = false
    @State private var search = ""

    var body: some View {
        Group {
            if let source = sources.sources[url] {
                // Nothing to choose from: straight to the app
                if source.apps.count == 1 && !showsSource {
                    SourceAppView(app: source.apps[0], source: source, sourceURL: url, isOnlyApp: true)
                } else {
                    list(source)
                }
            } else if let error = sources.errors[url] {
                List { ErrorRow(message: error) }
            } else {
                ProgressView()
            }
        }
        .navigationTitle(sources.sources[url]?.name ?? "Source")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await sources.refresh(url) }
    }

    private func apps(_ source: Source) -> [SourceApp] {
        // Featured apps first, as the source asks
        let featured = Set(source.featuredApps)
        let ordered = source.apps.filter { featured.contains($0.bundleId) }
            + source.apps.filter { !featured.contains($0.bundleId) }
        guard !search.isEmpty else { return ordered }
        return ordered.filter {
            $0.name.localizedCaseInsensitiveContains(search)
                || ($0.developerName ?? "").localizedCaseInsensitiveContains(search)
        }
    }

    private func list(_ source: Source) -> some View {
        List {
            if search.isEmpty {
                Section {
                    SourceHeader(source: source, url: url)
                        .listRowInsets(EdgeInsets())
                }
                Section {
                    Toggle("Update Its Apps Automatically", isOn: Binding(
                        get: { sources.updatesAutomatically(url) },
                        set: { sources.setUpdatesAutomatically($0, for: url) }
                    ))
                } footer: {
                    Text(sources.updatesAutomatically(url)
                         ? "Apps you get from \(source.name) update by themselves, unless one is set not to."
                         : "Apps from \(source.name) won't update by themselves. You can still update each from its page or the library.")
                }
                if !source.news.isEmpty {
                    Section("News") {
                        NewsStrip(source: source, url: url)
                            .listRowInsets(EdgeInsets())
                            .listRowBackground(Color.clear)
                    }
                }
            }
            Section(search.isEmpty ? "Apps" : "") {
                ForEach(apps(source), id: \.bundleId) { app in
                    NavigationLink {
                        SourceAppView(app: app, source: source, sourceURL: url)
                    } label: {
                        SourceAppRow(app: app)
                    }
                    .contextMenu { SourceAppMenu(app: app, source: source) }
                }
            }
            if !source.warnings.isEmpty {
                Section("Skipped") {
                    ForEach(source.warnings, id: \.self) { warning in
                        Text(warning).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .searchable(text: $search, prompt: "Search \(source.name)")
        .tint(Color(hex: source.tintColor))
    }
}

/// The source's banner, icon, name, description and website
private struct SourceHeader: View {
    let source: Source
    let url: String
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ZStack(alignment: .bottomLeading) {
                banner
                    .frame(height: 140)
                    .frame(maxWidth: .infinity)
                    .clipped()
                LinearGradient(colors: [.clear, .black.opacity(0.55)], startPoint: .center, endPoint: .bottom)
                HStack(alignment: .bottom, spacing: 12) {
                    RemoteIconView(url: source.iconUrl, size: 64)
                        .shadow(radius: 6)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(source.name)
                            .font(.title3.weight(.bold))
                            .foregroundStyle(.white)
                            .lineLimit(2)
                        Text("\(source.apps.count) app\(source.apps.count == 1 ? "" : "s") · \(URL(string: url)?.host() ?? url)")
                            .font(.caption)
                            .foregroundStyle(.white.opacity(0.85))
                            .lineLimit(1)
                    }
                }
                .padding(14)
            }
            .frame(height: 140)

            VStack(alignment: .leading, spacing: 8) {
                if let subtitle = source.subtitle {
                    Text(subtitle).font(.subheadline.weight(.semibold))
                }
                if let description = source.description, description != source.subtitle {
                    Text(description)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(expanded ? nil : 3)
                    if !expanded && description.count > 160 {
                        Button("more") { expanded = true }
                            .font(.subheadline.weight(.semibold))
                            .buttonStyle(.borderless)
                    }
                }
                if let website = source.website, let link = URL(string: website) {
                    Link(destination: link) {
                        Label(link.host() ?? website, systemImage: "safari")
                            .font(.subheadline)
                    }
                    .buttonStyle(.borderless)
                }
            }
            .padding(14)
        }
    }

    @ViewBuilder
    private var banner: some View {
        let tint = Color(hex: source.tintColor) ?? .accentColor
        if let header = source.headerUrl.flatMap(URL.init(string:)) {
            AsyncImage(url: header) { image in
                image.resizable().scaledToFill()
            } placeholder: {
                tint.opacity(0.6)
            }
        } else {
            LinearGradient(colors: [tint, tint.opacity(0.55)], startPoint: .topLeading, endPoint: .bottomTrailing)
        }
    }
}

/// The source's news, newest first, as cards to scroll through
private struct NewsStrip: View {
    let source: Source
    let url: String

    private var news: [NewsItem] {
        source.news.sorted { ($0.date ?? "") > ($1.date ?? "") }
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 12) {
                ForEach(Array(news.enumerated()), id: \.offset) { _, item in
                    card(item)
                }
            }
            .scrollTargetLayout()
        }
        .scrollTargetBehavior(.viewAligned)
        .contentMargins(.horizontal, 16, for: .scrollContent)
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private func card(_ item: NewsItem) -> some View {
        let content = NewsCard(item: item, tint: Color(hex: source.tintColor) ?? .accentColor)
            .contextMenu {
                if let link = item.url.flatMap(URL.init(string:)) {
                    Link(destination: link) { Label("Open Link", systemImage: "safari") }
                    Button("Copy Link", systemImage: "link") { UIPasteboard.general.string = link.absoluteString }
                    ShareLink(item: link) { Label("Share", systemImage: "square.and.arrow.up") }
                }
                Button("Copy Text", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = [item.title, item.caption].compactMap { $0 }.joined(separator: "\n")
                }
            }
        if let appId = item.appId, let app = source.apps.first(where: { $0.bundleId == appId }) {
            NavigationLink {
                SourceAppView(app: app, source: source, sourceURL: url)
            } label: { content }
                .buttonStyle(.plain)
        } else if let link = item.url.flatMap(URL.init(string:)) {
            Link(destination: link) { content }
                .buttonStyle(.plain)
        } else {
            content
        }
    }
}

private struct NewsCard: View {
    let item: NewsItem
    let tint: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Group {
                if let image = item.imageUrl.flatMap(URL.init(string:)) {
                    AsyncImage(url: image) { image in
                        image.resizable().scaledToFill()
                    } placeholder: {
                        tint.opacity(0.4)
                    }
                } else {
                    LinearGradient(colors: [tint, tint.opacity(0.5)], startPoint: .topLeading, endPoint: .bottomTrailing)
                }
            }
            .frame(width: 260, height: 110)
            .clipped()
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(2)
                if let caption = item.caption {
                    Text(caption).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
                if let date = item.date.flatMap(SourceDate.parse) {
                    Text(date, format: .dateTime.day().month().year())
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(10)
            .frame(width: 260, alignment: .leading)
        }
        .frame(width: 260, height: 200, alignment: .top)
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(.quaternary, lineWidth: 0.5)
        }
    }
}

private struct SourceAppRow: View {
    let app: SourceApp

    var body: some View {
        HStack(spacing: 12) {
            RemoteIconView(url: app.iconUrl)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.name).font(.headline).lineLimit(1)
                if let subtitle = app.subtitle ?? app.developerName {
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Text(versionLine)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.vertical, 2)
    }

    private var versionLine: String {
        let latest = app.versions[0]
        if let size = latest.size {
            return "\(latest.version) · \(ByteCountFormatter.file(size))"
        }
        return latest.version
    }
}
