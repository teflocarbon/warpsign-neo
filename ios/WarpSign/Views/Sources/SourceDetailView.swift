import SwiftUI

struct SourceDetailView: View {
    @Environment(SourcesStore.self) private var sources
    let url: String
    @State private var search = ""

    var body: some View {
        Group {
            if let source = sources.sources[url] {
                // Nothing to choose from: straight to the app
                if source.apps.count == 1 {
                    SourceAppView(app: source.apps[0], source: source, isOnlyApp: true)
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
            if search.isEmpty, let subtitle = source.subtitle ?? source.description {
                Section {
                    Text(subtitle)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
            Section {
                ForEach(apps(source), id: \.bundleId) { app in
                    NavigationLink {
                        SourceAppView(app: app, source: source)
                    } label: {
                        SourceAppRow(app: app)
                    }
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
