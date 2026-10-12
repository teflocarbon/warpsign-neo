import SwiftUI
import UniformTypeIdentifiers

/// Imported IPAs, ready to sign
struct LibraryView: View {
    @Environment(LibraryStore.self) private var library
    @State private var showingImporter = false
    @State private var search = ""
    @State private var path: [UUID] = Demo.isOn && Demo.screen == .appDetail ? [Demo.appIds[0]] : []

    static let ipaType = UTType("com.apple.itunes.ipa")
        ?? UTType(filenameExtension: "ipa")
        ?? .data

    private var apps: [LibraryApp] {
        guard !search.isEmpty else { return library.apps }
        return library.apps.filter {
            $0.displayName.localizedCaseInsensitiveContains(search)
                || $0.bundleId.localizedCaseInsensitiveContains(search)
        }
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if !library.importing.isEmpty {
                    Section {
                        ForEach(library.importing, id: \.self) { name in
                            HStack(spacing: 12) {
                                ProgressView()
                                Text("Importing \(name)…")
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if let error = library.importError {
                    Section { ErrorRow(message: error) }
                }
                if search.isEmpty {
                    MigrationBanner()
                }
                ForEach(apps) { app in
                    NavigationLink(value: app.id) {
                        LibraryRow(app: app)
                    }
                }
                .onDelete { offsets in
                    for index in offsets { library.delete(apps[index]) }
                }
            }
            .overlay {
                if library.apps.isEmpty && library.importing.isEmpty {
                    ContentUnavailableView {
                        Label("No Apps", systemImage: "square.stack.3d.up")
                    } description: {
                        Text("Import an IPA from Files, share one to WarpSign, or download one from a source.")
                    } actions: {
                        Button("Import IPA") { showingImporter = true }
                            .buttonStyle(.borderedProminent)
                    }
                }
            }
            .searchable(text: $search, prompt: "Search apps")
            .navigationTitle("Library")
            .navigationDestination(for: UUID.self) { id in
                AppDetailView(appId: id)
            }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Import IPA", systemImage: "plus") {
                        showingImporter = true
                    }
                }
            }
            .fileImporter(
                isPresented: $showingImporter,
                allowedContentTypes: [Self.ipaType, .zip],
                allowsMultipleSelection: true
            ) { result in
                guard case .success(let urls) = result else { return }
                Task {
                    for url in urls {
                        await library.importFile(url)
                    }
                }
            }
        }
    }
}

struct LibraryRow: View {
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp

    var body: some View {
        HStack(spacing: 12) {
            AppIconView(image: library.icon(for: app.id))
            VStack(alignment: .leading, spacing: 3) {
                Text(app.displayName)
                    .font(.headline)
                    .lineLimit(1)
                Text("\(app.versionText) · \(ByteCountFormatter.file(app.fileSize))")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                Text(app.bundleId)
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
            VStack(alignment: .trailing, spacing: 4) {
                if app.isEncrypted {
                    Badge("Encrypted", color: .red)
                }
                if !app.signedCopies.isEmpty {
                    Badge("Signed", color: .green)
                }
            }
        }
        .padding(.vertical, 2)
    }
}
