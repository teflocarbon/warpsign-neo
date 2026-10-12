import SwiftUI
import UniformTypeIdentifiers

/// Imported IPAs, ready to sign
struct LibraryView: View {
    @Environment(LibraryStore.self) private var library
    @State private var showingImporter = false
    @State private var search = ""
    @State private var companionDismissed = UserDefaults.standard.bool(forKey: BundledCompanion.dismissedKey)
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
                    UpdatesSection()
                    companionCard
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
                // Any file: the type iOS gives .ipa files depends on which installed
                // app declared it, and a mismatch greys them out. Checked after.
                allowedContentTypes: [.data],
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

/// WarpSign Companion, bundled inside WarpSign by CI: a second app that
/// can install WarpSign over WarpSign, which WarpSign can't do itself
enum BundledCompanion {
    static var url: URL? {
        Bundle.main.url(forResource: "WarpSignCompanion", withExtension: "ipa")
    }

    /// The library origin it's added with, to know it's there
    static let origin = "Bundled with WarpSign"
    static let dismissedKey = "companion.dismissed"
}

extension LibraryView {
    /// Offered once, until it's set up or dismissed
    @ViewBuilder
    var companionCard: some View {
        if let url = BundledCompanion.url, !companionDismissed,
           !library.apps.contains(where: { $0.origin == BundledCompanion.origin }) {
            Section {
                Label {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Set Up WarpSign Companion").font(.headline)
                        Text("A small second app that installs WarpSign's own updates, which WarpSign can't install over itself. Sign and install it once.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                } icon: {
                    Image(systemName: "arrow.triangle.2.circlepath.circle.fill")
                        .foregroundStyle(.tint)
                }
                Button("Set Up Companion", systemImage: "plus.app") {
                    Task {
                        if let app = await library.importFile(url, origin: BundledCompanion.origin) {
                            path.append(app.id)
                        }
                    }
                }
                Button("Not Now", role: .cancel) {
                    companionDismissed = true
                    UserDefaults.standard.set(true, forKey: BundledCompanion.dismissedKey)
                }
                .foregroundStyle(.secondary)
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
                UpdateOriginLabel(app: app)
            }
            Spacer(minLength: 0)
            VStack(alignment: .trailing, spacing: 4) {
                if Updater.shared.available[app.id] != nil {
                    Badge("Update", color: .blue)
                }
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
