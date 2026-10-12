import SwiftUI
import UniformTypeIdentifiers

/// Imported IPAs, ready to sign
struct LibraryView: View {
    @Environment(LibraryStore.self) private var library
    @Environment(DeviceStore.self) private var device
    @State private var showingImporter = false
    @State private var showingLink = false
    @State private var linking: LibraryApp?
    @State private var removing: LibraryApp?
    @State private var showingActivity = false
    @State private var search = ""
    @AppStorage("library.filter") private var filter: LibraryFilter = .all
    @State private var path: [UUID] = Demo.isOn && Demo.screen == .appDetail ? [Demo.appIds[0]] : []

    static let ipaType = UTType("com.apple.itunes.ipa")
        ?? UTType(filenameExtension: "ipa")
        ?? .data

    /// The library's apps; WarpSign Companion lives in Settings instead
    private var allApps: [LibraryApp] {
        library.apps.filter { $0.updateOrigin != .bundled }
    }

    /// The filter in force: one that can't be answered now (installed,
    /// before the device is asked) shows everything
    private var activeFilter: LibraryFilter {
        filter.isAvailable(device: device) ? filter : .all
    }

    /// What's shown: filtered, then searched
    private var apps: [LibraryApp] {
        let apps = allApps.filter { activeFilter.includes($0, device: device) }
        guard !search.isEmpty else { return apps }
        return apps.filter {
            $0.displayName.localizedCaseInsensitiveContains(search)
                || $0.bundleId.localizedCaseInsensitiveContains(search)
        }
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if !allApps.isEmpty {
                    Section {
                        LibraryFilterBar(selection: $filter, apps: allApps)
                            .listRowInsets(EdgeInsets())
                            .listRowBackground(Color.clear)
                    }
                    .listSectionSpacing(.compact)
                }
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
                // While something's going, or needs a look: Activity has the rest
                if ActivityStrip.isShown {
                    Section {
                        ActivityStrip { showingActivity = true }
                    }
                }
                if let error = library.importError {
                    Section { ErrorRow(message: error) }
                }
                if search.isEmpty && activeFilter == .all {
                    MigrationBanner()
                    UpdatesSection()
                }
                ForEach(apps) { app in
                    NavigationLink(value: app.id) {
                        LibraryRow(app: app)
                    }
                    .contextMenu {
                        LibraryAppMenu(
                            app: app,
                            open: { path.append(app.id) },
                            link: { linking = app },
                            remove: { removing = app }
                        )
                    }
                }
                .onDelete { offsets in
                    for index in offsets { library.delete(apps[index]) }
                }
            }
            .overlay {
                if apps.isEmpty && !allApps.isEmpty && activeFilter != .all && search.isEmpty {
                    ContentUnavailableView {
                        Label("Nothing Here", systemImage: activeFilter.symbol)
                    } description: {
                        Text("No apps are \(activeFilter.title.lowercased()) right now.")
                    } actions: {
                        Button("Show All") { withAnimation(.snappy) { filter = .all } }
                    }
                } else if library.apps.isEmpty && library.importing.isEmpty && DownloadManager.shared.items.isEmpty {
                    ContentUnavailableView {
                        Label("No Apps", systemImage: "square.stack.3d.up")
                    } description: {
                        Text("Import an IPA from Files, share one to WarpSign, download one from a link, or get one from a source.")
                    } actions: {
                        Button("Import from Files") { showingImporter = true }
                            .buttonStyle(.borderedProminent)
                        Button("Download from Link") { showingLink = true }
                    }
                }
            }
            .searchable(text: $search, prompt: "Search apps")
            // Pulling asks the device what's installed (turning the VPN on),
            // and the sources for updates
            .refreshable {
                async let installed: Void = device.refreshInstalled()
                async let updates: Void = Updater.shared.check()
                _ = await (installed, updates)
            }
            .task { await device.refreshInstalledIfDue() }
            .navigationTitle("Library")
            .navigationDestination(for: UUID.self) { id in
                AppDetailView(appId: id)
            }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    ActivityButton { showingActivity = true }
                }
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        Button("Import from Files", systemImage: "folder") {
                            showingImporter = true
                        }
                        Button("Download from Link", systemImage: "link") {
                            showingLink = true
                        }
                    } label: {
                        Label("Add App", systemImage: "plus")
                    }
                }
            }
            .modifier(LinkDownloadAlert(isPresented: $showingLink))
            .sheet(isPresented: $showingActivity) {
                ActivitySheet(open: { id in path = [id] })
                    .environment(library)
                    .environment(device)
            }
            .sheet(item: $linking) { app in
                LinkSourceView(app: app)
            }
            .confirmationDialog(
                removing.map { "Remove \($0.displayName)?" } ?? "",
                isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                titleVisibility: .visible,
                presenting: removing
            ) { app in
                Button("Remove", role: .destructive) { library.delete(app) }
            } message: { _ in
                Text("The IPA and its signed copies are deleted from WarpSign.")
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
}

struct LibraryRow: View {
    @Environment(LibraryStore.self) private var library
    @Environment(DeviceStore.self) private var device
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
                switch device.compareInstalled(app) {
                case .same:
                    Badge("Installed", color: .green)
                case .libraryNewer:
                    if app.currentCopy == nil {
                        Badge("Not Signed Yet", color: .orange)
                    } else {
                        Badge("Not Installed Yet", color: .orange)
                    }
                case .installedNewer:
                    Badge("Newer Installed", color: .secondary)
                case nil:
                    if device.isMissing(app) {
                        Badge("Not Installed", color: .secondary)
                    } else if app.needsSigning == true {
                        Badge("Not Signed Yet", color: .orange)
                    } else if !app.signedCopies.isEmpty {
                        Badge("Signed", color: .green)
                    }
                }
            }
        }
        .padding(.vertical, 2)
    }
}
