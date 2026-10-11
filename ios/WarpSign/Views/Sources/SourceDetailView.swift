import SwiftUI

struct SourceDetailView: View {
    @Environment(SourcesStore.self) private var sources
    let url: String
    @State private var search = ""

    var body: some View {
        Group {
            if let source = sources.sources[url] {
                list(source)
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

struct SourceAppView: View {
    @Environment(SourcesStore.self) private var sources
    let app: SourceApp
    let source: Source

    var body: some View {
        List {
            Section {
                HStack(spacing: 16) {
                    RemoteIconView(url: app.iconUrl, size: 84)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(app.name).font(.title2.weight(.semibold))
                        if let developer = app.developerName {
                            Text(developer).foregroundStyle(.secondary)
                        }
                        DownloadButton(version: app.versions[0], app: app, source: source)
                    }
                }
                .padding(.vertical, 6)
            }

            if let error = sources.downloadError {
                Section { ErrorRow(message: error) }
            }

            if !app.screenshotUrls.isEmpty {
                Section {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 12) {
                            ForEach(app.screenshotUrls, id: \.self) { url in
                                AsyncImage(url: URL(string: url)) { image in
                                    image.resizable().scaledToFit()
                                } placeholder: {
                                    Color.gray.opacity(0.15)
                                }
                                .frame(height: 360)
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                            }
                        }
                        .padding(.vertical, 8)
                    }
                    .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
                }
            }

            if let description = app.description {
                Section("About") {
                    Text(description).font(.callout)
                }
            }

            Section("Versions") {
                ForEach(app.versions, id: \.downloadUrl) { version in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(version.version).font(.headline)
                            if let size = version.size {
                                Text(ByteCountFormatter.file(size))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            DownloadButton(version: version, app: app, source: source)
                        }
                        if let date = version.date {
                            Text(date.prefix(10)).font(.caption).foregroundStyle(.secondary)
                        }
                        if let notes = version.notes {
                            Text(notes).font(.caption).foregroundStyle(.secondary).lineLimit(4)
                        }
                        if let minOs = version.minOs {
                            Text("Requires iOS \(minOs)").font(.caption2).foregroundStyle(.tertiary)
                        }
                    }
                    .padding(.vertical, 2)
                }
            }

            Section {
                LabeledContent("Bundle ID") {
                    Text(app.bundleId).font(.caption.monospaced())
                }
                LabeledContent("Source", value: source.name)
            }
        }
        .navigationTitle(app.name)
        .navigationBarTitleDisplayMode(.inline)
        .tint(Color(hex: app.tintColor) ?? Color(hex: source.tintColor))
    }
}

/// Downloads a version into the library, showing progress
private struct DownloadButton: View {
    @Environment(SourcesStore.self) private var sources
    @Environment(LibraryStore.self) private var library
    let version: AppVersion
    let app: SourceApp
    let source: Source

    var body: some View {
        if let progress = sources.downloads[version.downloadUrl] {
            ProgressView(value: progress)
                .progressViewStyle(.circular)
                .frame(width: 28, height: 28)
        } else {
            Button {
                Task { await sources.download(version, of: app, from: source, into: library) }
            } label: {
                Text("Get")
                    .font(.subheadline.weight(.semibold))
                    .padding(.horizontal, 6)
            }
            .buttonStyle(.bordered)
            .buttonBorderShape(.capsule)
        }
    }
}
