import SwiftUI
import UIKit

/// AltStore-format sources to download apps from
struct SourcesView: View {
    @Environment(SourcesStore.self) private var sources
    @Environment(LibraryStore.self) private var library
    @State private var adding = false
    @State private var newURL = ""
    @State private var editing: String?
    @State private var editedURL = ""
    @State private var removing: String?
    @State private var addError: String?
    @State private var checking = false
    @State private var path: [String] = Demo.isOn && Demo.screen == .sourceDetail ? [Demo.sourceURL] : []

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if let addError {
                    Section { ErrorRow(message: addError) }
                }
                if checking {
                    Section {
                        HStack(spacing: 12) {
                            ProgressView()
                            Text("Checking the source…").foregroundStyle(.secondary)
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
            .overlay {
                if sources.urls.isEmpty && !checking {
                    ContentUnavailableView {
                        Label("No Sources", systemImage: "globe")
                    } description: {
                        Text("Add an AltStore-compatible source to browse and download its apps. AltStore, SideStore and Feather links work too.")
                    } actions: {
                        Button("Add Source") { adding = true }
                            .buttonStyle(.borderedProminent)
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
                    Button("Add Source", systemImage: "plus") { adding = true }
                }
            }
            .alert("Add Source", isPresented: $adding) {
                TextField("https://example.com/apps.json", text: $newURL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Cancel", role: .cancel) { newURL = "" }
                Button("Add") {
                    add(newURL)
                    newURL = ""
                }
            } message: {
                Text("The source's address, or an AltStore, SideStore or Feather link to it.")
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
                    add(url)
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
        }
    }

    @ViewBuilder
    private func actions(_ url: String) -> some View {
        Button("Copy Link", systemImage: "doc.on.doc") { UIPasteboard.general.string = url }
        if let link = URL(string: url) {
            ShareLink(item: link) { Label("Share", systemImage: "square.and.arrow.up") }
        }
        Button("Edit", systemImage: "pencil") { edit(url) }
        Button("Refresh", systemImage: "arrow.clockwise") {
            Task { await sources.refresh(url) }
        }
        if let website = sources.sources[url]?.website.flatMap(URL.init(string:)) {
            Link(destination: website) { Label("Website", systemImage: "safari") }
        }
        Divider()
        Button("Remove", systemImage: "trash", role: .destructive) { removing = url }
    }

    private func add(_ text: String) {
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        checking = true
        Task {
            defer { checking = false }
            do {
                try await sources.add(text)
                addError = nil
                Updater.shared.evaluate()
            } catch {
                addError = error.userMessage
            }
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
