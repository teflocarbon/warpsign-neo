import SwiftUI

/// AltStore-format sources to download apps from
struct SourcesView: View {
    @Environment(SourcesStore.self) private var sources
    @State private var adding = false
    @State private var newURL = ""
    @State private var addError: String?
    @State private var checking = false

    var body: some View {
        NavigationStack {
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
                }
                .onDelete { sources.remove(at: $0) }
            }
            .overlay {
                if sources.urls.isEmpty && !checking {
                    ContentUnavailableView {
                        Label("No Sources", systemImage: "globe")
                    } description: {
                        Text("Add an AltStore-compatible source to browse and download its apps.")
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
                Button("Add") { add() }
            } message: {
                Text("The address of the source's JSON.")
            }
        }
    }

    private func add() {
        let url = newURL
        newURL = ""
        guard !url.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        checking = true
        Task {
            defer { checking = false }
            do {
                try await sources.add(url)
                addError = nil
            } catch {
                addError = error.userMessage
            }
        }
    }
}

private struct SourceRow: View {
    @Environment(SourcesStore.self) private var sources
    let url: String

    var body: some View {
        HStack(spacing: 12) {
            RemoteIconView(url: sources.sources[url]?.iconUrl, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                Text(sources.sources[url]?.name ?? URL(string: url)?.host() ?? url)
                    .font(.headline)
                if let error = sources.errors[url] {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(2)
                } else if let source = sources.sources[url] {
                    Text("\(source.apps.count) app\(source.apps.count == 1 ? "" : "s")")
                        .font(.caption)
                        .foregroundStyle(.secondary)
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
    }
}
