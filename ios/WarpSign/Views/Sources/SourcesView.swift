import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// AltStore-format sources to download apps from, and every app in them
/// to search
struct SourcesView: View {
    @Environment(SourcesStore.self) private var sources
    @Environment(LibraryStore.self) private var library
    @State private var adding = false
    @State private var newURL = ""
    @State private var editing: String?
    @State private var editedURL = ""
    @State private var removing: String?
    @State private var importingFile = false
    @State private var message: String?
    @State private var addError: String?
    @State private var checking = false
    /// Searching every source's apps
    @State private var query = ""
    @State private var path: [String] = Demo.isOn && Demo.screen == .sourceDetail ? [Demo.sourceURL] : []

    /// An app found by searching, and where it's from
    struct Result: Identifiable {
        let url: String
        let source: Source
        let app: SourceApp
        var id: String { url + "\n" + app.bundleId }
    }

    /// Every source's apps matching the search: name matches first
    private var results: [Result] {
        let query = query.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else { return [] }
        let all = sources.urls.flatMap { url -> [Result] in
            guard let source = sources.sources[url] else { return [] }
            return source.apps.compactMap { app in
                let fields = [app.name, app.subtitle, app.developerName, app.bundleId].compactMap { $0 }
                guard fields.contains(where: { $0.localizedCaseInsensitiveContains(query) }) else { return nil }
                return Result(url: url, source: source, app: app)
            }
        }
        return all.sorted { a, b in
            let aName = a.app.name.localizedCaseInsensitiveContains(query)
            let bName = b.app.name.localizedCaseInsensitiveContains(query)
            if aName != bName { return aName }
            return a.app.name.localizedCaseInsensitiveCompare(b.app.name) == .orderedAscending
        }
    }

    /// The sources, one link per line, to share or copy
    private var linksText: String { sources.urls.joined(separator: "\n") }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if query.isEmpty {
                    sourceList
                } else {
                    searchResults
                }
            }
            .searchable(text: $query, prompt: "Search all sources")
            .overlay {
                if !query.isEmpty && results.isEmpty {
                    ContentUnavailableView.search(text: query)
                } else if sources.urls.isEmpty && !checking && query.isEmpty {
                    ContentUnavailableView {
                        Label("No Sources", systemImage: "globe")
                    } description: {
                        Text("Add an AltStore-compatible source to browse and download its apps. AltStore, SideStore and Feather links work too.")
                    } actions: {
                        Button("Add Source") { adding = true }
                            .buttonStyle(.borderedProminent)
                        Button("Import Sources from File") { importingFile = true }
                    }
                }
            }
            .navigationTitle("Sources")
            .navigationDestination(for: String.self) { url in
                SourceDetailView(url: url)
            }
            .refreshable { await sources.refreshAll() }
            .task { await sources.refreshAll() }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        Button("Add Source", systemImage: "plus") { adding = true }
                        Button("Import Sources from File", systemImage: "doc.badge.plus") { importingFile = true }
                        if !sources.urls.isEmpty {
                            Divider()
                            ShareLink(item: linksText, subject: Text("WarpSign Sources")) {
                                Label("Share Sources", systemImage: "square.and.arrow.up")
                            }
                            Button("Copy All Links", systemImage: "doc.on.doc") {
                                UIPasteboard.general.string = linksText
                                message = "Copied \(sources.urls.count) source link\(sources.urls.count == 1 ? "" : "s")."
                            }
                        }
                    } label: {
                        Label("Add Source", systemImage: "plus")
                    }
                }
            }
            .alert("Add Source", isPresented: $adding) {
                TextField("https://example.com/apps.json", text: $newURL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Cancel", role: .cancel) { newURL = "" }
                Button("Add") {
                    addAll(newURL)
                    newURL = ""
                }
            } message: {
                Text("The source's address, or an AltStore, SideStore or Feather link to it. Paste several, separated by spaces or new lines, to add them all.")
            }
            .alert("Edit Source", isPresented: Binding(get: { editing != nil }, set: { if !$0 { editing = nil } })) {
                TextField("https://example.com/apps.json", text: $editedURL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Cancel", role: .cancel) { editing = nil }
                Button("Save") { save() }
            } message: {
                Text("Apps from it keep following it at its new address.")
            }
            .alert(
                "Add Source?",
                isPresented: Binding(get: { sources.proposed != nil }, set: { if !$0 { sources.proposed = nil } }),
                presenting: sources.proposed
            ) { url in
                Button("Cancel", role: .cancel) { sources.proposed = nil }
                Button("Add") {
                    sources.proposed = nil
                    addAll(url)
                }
            } message: { url in
                Text(url)
            }
            .confirmationDialog(
                removing.map { "Remove \(sources.sources[$0]?.name ?? "this source")?" } ?? "",
                isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                titleVisibility: .visible,
                presenting: removing
            ) { url in
                Button("Remove", role: .destructive) { remove(url) }
            } message: { url in
                let count = library.apps.filter { $0.sourceURL == url }.count
                Text(count == 0
                     ? "Its apps in your library aren't touched."
                     : "\(count) app\(count == 1 ? "" : "s") in your library stop updating from it, and stay in the library.")
            }
            .fileImporter(isPresented: $importingFile, allowedContentTypes: [.data, .text, .json]) { result in
                guard case .success(let file) = result else { return }
                importFile(file)
            }
        }
    }

    @ViewBuilder
    private var sourceList: some View {
        if let addError {
            Section { ErrorRow(message: addError) }
        }
        if let message {
            Section {
                Label(message, systemImage: "checkmark.circle").foregroundStyle(.green)
            }
        }
        if checking {
            Section {
                HStack(spacing: 12) {
                    ProgressView()
                    Text("Checking sources…").foregroundStyle(.secondary)
                }
            }
        }
        ForEach(sources.urls, id: \.self) { url in
            NavigationLink(value: url) {
                SourceRow(url: url)
            }
            .contextMenu { actions(url) }
            .swipeActions(edge: .trailing) {
                Button("Remove", systemImage: "trash", role: .destructive) { removing = url }
                Button("Edit", systemImage: "pencil") { edit(url) }
                    .tint(.orange)
            }
            .swipeActions(edge: .leading) {
                Button("Copy Link", systemImage: "doc.on.doc") { UIPasteboard.general.string = url }
                    .tint(.blue)
            }
        }
    }

    @ViewBuilder
    private var searchResults: some View {
        let found = results
        // By source, in the order the sources are listed
        ForEach(sources.urls.filter { url in found.contains { $0.url == url } }, id: \.self) { url in
            Section(sources.sources[url]?.name ?? url) {
                ForEach(found.filter { $0.url == url }) { result in
                    NavigationLink {
                        SourceAppView(app: result.app, source: result.source, sourceURL: result.url)
                    } label: {
                        SourceAppRow(app: result.app)
                    }
                    .contextMenu { SourceAppMenu(app: result.app, source: result.source) }
                }
            }
        }
    }

    @ViewBuilder
    private func actions(_ url: String) -> some View {
        Button("Copy Link", systemImage: "doc.on.doc") { UIPasteboard.general.string = url }
        if let link = URL(string: url) {
            ShareLink(item: link) { Label("Share", systemImage: "square.and.arrow.up") }
        }
        Button("Edit", systemImage: "pencil") { edit(url) }
        if sources.updatesAutomatically(url) {
            Button("Don't Update Its Apps Automatically", systemImage: "pause.circle") {
                sources.setUpdatesAutomatically(false, for: url)
            }
        } else {
            Button("Update Its Apps Automatically", systemImage: "arrow.triangle.2.circlepath") {
                sources.setUpdatesAutomatically(true, for: url)
            }
        }
        Button("Refresh", systemImage: "arrow.clockwise") {
            Task { await sources.refresh(url) }
        }
        if let website = sources.sources[url]?.website.flatMap(URL.init(string:)) {
            Link(destination: website) { Label("Website", systemImage: "safari") }
        }
        Divider()
        Button("Remove", systemImage: "trash", role: .destructive) { removing = url }
    }

    /// Source links in text: web, AltStore, SideStore, Feather and
    /// WarpSign links (from a pasted list, a shared file, JSON), or bare
    /// addresses separated by spaces or lines
    static func links(in text: String) -> [String] {
        let pattern = #"(?i)\b(?:https?|altstore|sidestore|feather|warpsign)://[^\s"'<>,\]\)]+"#
        let regex = try? NSRegularExpression(pattern: pattern)
        let range = NSRange(text.startIndex..., in: text)
        let found = regex?.matches(in: text, range: range).compactMap { match in
            Range(match.range, in: text).map { String(text[$0]) }
        } ?? []
        if !found.isEmpty { return found }
        return text.split(whereSeparator: { $0.isWhitespace || $0 == "," }).map(String.init)
    }

    /// Add every source in `text`, one after another, saying how it went
    private func addAll(_ text: String) {
        let links = Self.links(in: text)
        guard !links.isEmpty else { return }
        checking = true
        message = nil
        Task {
            defer { checking = false }
            var added = 0
            var failures: [String] = []
            for link in links {
                let before = sources.urls.count
                do {
                    try await sources.add(link)
                    if sources.urls.count > before { added += 1 }
                } catch {
                    failures.append(links.count == 1 ? error.userMessage : "\(link): \(error.userMessage)")
                }
            }
            addError = failures.isEmpty ? nil : failures.joined(separator: "\n")
            if links.count > 1 || added == 0 && failures.isEmpty {
                message = added == 0 && failures.isEmpty
                    ? "Already added."
                    : "Added \(added) source\(added == 1 ? "" : "s")."
            }
            Updater.shared.evaluate()
        }
    }

    private func importFile(_ file: URL) {
        do {
            let text = try withSecurityScopedAccess(to: file) {
                try String(contentsOf: file, encoding: .utf8)
            }
            let links = Self.links(in: text)
            guard !links.isEmpty else {
                addError = "\(file.lastPathComponent) has no source links in it."
                return
            }
            addAll(links.joined(separator: "\n"))
        } catch {
            addError = "Couldn't read \(file.lastPathComponent): \(error.userMessage)"
        }
    }

    private func edit(_ url: String) {
        editedURL = url
        editing = url
    }

    private func save() {
        guard let old = editing else { return }
        let text = editedURL
        editing = nil
        checking = true
        Task {
            defer { checking = false }
            do {
                let new = try await sources.replace(old, with: text)
                library.moveSource(from: old, to: new)
                addError = nil
                Updater.shared.evaluate()
            } catch {
                addError = error.userMessage
            }
        }
    }

    private func remove(_ url: String) {
        guard let index = sources.urls.firstIndex(of: url) else { return }
        library.unlinkSource(url)
        sources.remove(at: IndexSet(integer: index))
        Updater.shared.evaluate()
    }
}

private struct SourceRow: View {
    @Environment(SourcesStore.self) private var sources
    let url: String

    var body: some View {
        HStack(spacing: 12) {
            RemoteIconView(url: sources.sources[url]?.iconUrl, size: 48)
            VStack(alignment: .leading, spacing: 2) {
                Text(sources.sources[url]?.name ?? URL(string: url)?.host() ?? url)
                    .font(.headline)
                    .lineLimit(1)
                if let error = sources.errors[url] {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(2)
                } else if let source = sources.sources[url] {
                    if let subtitle = source.subtitle {
                        Text(subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    Text("\(source.apps.count) app\(source.apps.count == 1 ? "" : "s") · \(URL(string: url)?.host() ?? url)")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                    if !sources.updatesAutomatically(url) {
                        Label("Its apps won't update by themselves", systemImage: "pause.circle")
                            .font(.caption2)
                            .foregroundStyle(.orange)
                    }
                } else {
                    Text(url)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if sources.loading.contains(url) {
                ProgressView()
            }
        }
        .padding(.vertical, 2)
    }
}
