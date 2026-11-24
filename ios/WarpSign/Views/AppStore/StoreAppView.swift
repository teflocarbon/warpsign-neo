import SwiftUI

/// An App Store app, on the shared app page (AppPage.swift), as the
/// catalog describes it: Get downloads it into the library
struct StoreAppView: View {
    @Environment(LibraryStore.self) private var library
    let app: CatalogApp
    /// The store it's from, when not the one being browsed
    var country: String? = nil

    private var store: AppStoreModel { .shared }

    private var countryName: String { StoreCountry.name(country ?? store.country) }

    var body: some View {
        AppPage(iconURL: app.iconURL) { large in
            StoreGetButton(app: app, large: large)
        } content: {
            AppPageHeader(iconURL: app.iconURL, name: app.name, subtitle: app.developer, stats: stats) {
                AppOriginLine(systemImage: "bag", text: "App Store · \(countryName)")
            } action: {
                StoreGetButton(app: app, large: true)
            } menu: {
                if let link = app.trackViewUrl, let url = URL(string: link) {
                    ShareLink(item: url)
                }
                CopyMenu(title: "Bundle ID", value: app.bundleId)
            }
            if let minimum = app.needsNewerSystem {
                Label {
                    Text("Needs iOS \(minimum) or later, and this \(UIDevice.current.model) has iOS \(UIDevice.current.systemVersion). You can get and sign it, but it won't install.")
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                }
                .font(.subheadline)
                .foregroundStyle(.orange)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .padding(.top, 12)
            }
            if let download = store.downloads[app.id] {
                StoreDownloadRow(download: download)
                    .padding(12)
                    .appCard()
                    .padding(.top, 12)
            }
            InLibrarySection(apps: library.apps.filter { $0.bundleId == app.bundleId })
            WhatsNewSection(version: app.version ?? "Latest", date: app.releaseDate, notes: app.releaseNotes) {
                NavigationLink("Version History") {
                    StoreVersionsView(app: app)
                }
            }
            AppScreenshots(urls: app.screenshots)
            if let description = app.description {
                AppPageSection("Description") {
                    ExpandableText(description)
                }
            }
            information
        }
    }

    private var stats: [AppStat] {
        var items: [AppStat] = []
        if let rating = app.averageUserRating, let count = app.userRatingCount, count > 0 {
            items.append(AppStat(
                label: "\(count.formatted(.number.notation(.compactName))) Ratings",
                value: rating.formatted(.number.precision(.fractionLength(1))),
                detail: String(repeating: "★", count: Int(rating.rounded()))
            ))
        } else if let version = app.version {
            items.append(.version(version, build: nil))
        }
        if let size = app.size {
            items.append(.size(size))
        }
        if let minimum = app.minimumOsVersion {
            items.append(.requires(minimum))
        }
        if let genre = app.primaryGenreName {
            items.append(AppStat(label: "Category", value: genre, detail: app.contentAdvisoryRating ?? ""))
        }
        return items
    }

    private var information: some View {
        AppPageSection("Information") {
            VStack(spacing: 0) {
                if let developer = app.developer {
                    InfoRow("Provider", developer)
                }
                InfoRow("Bundle ID", app.bundleId, monospaced: true)
                if let version = app.version {
                    InfoRow("Version", version)
                }
                if let size = app.size {
                    InfoRow("Size", ByteCountFormatter.file(size))
                }
                if let genre = app.primaryGenreName {
                    InfoRow("Category", genre)
                }
                if let minOs = app.minimumOsVersion {
                    InfoRow("Requires", "iOS \(minOs) or later")
                }
                if let age = app.contentAdvisoryRating {
                    InfoRow("Age Rating", age)
                }
                InfoRow("Price", app.isFree ? "Free" : app.getTitle)
                InfoRow("Store", countryName)
                InfoRow("App Store ID", String(app.id), monospaced: true)
                if let website = app.sellerUrl, let url = URL(string: website) {
                    InfoLinkRow(label: "Developer Website", url: url)
                }
                if let link = app.trackViewUrl, let url = URL(string: link) {
                    InfoLinkRow(label: "App Store", url: url, text: "View")
                }
            }
        }
    }
}

/// Get: downloads the app (or a version) into the library, showing
/// progress. Asks to sign in first.
struct StoreGetButton: View {
    @Environment(LibraryStore.self) private var library
    @Environment(AccountStore.self) private var account
    let app: CatalogApp
    /// An older version; the latest when nil
    var versionId: String? = nil
    var large = false
    @State private var signingIn = false

    private var store: AppStoreModel { .shared }

    private var download: StoreDownload? {
        store.downloads[app.id].flatMap { $0.isActive && $0.versionId == versionId ? $0 : nil }
    }

    var body: some View {
        if let download {
            if let fraction = download.fraction {
                ProgressView(value: fraction)
                    .progressViewStyle(.circular)
                    .frame(width: 28, height: 28)
            } else {
                ProgressView().frame(width: 28, height: 28)
            }
        } else {
            AppGetPill(title: versionId == nil ? app.getTitle : "Get", large: large) {
                if store.signedInAccount == nil {
                    signingIn = true
                } else {
                    store.download(
                        app, versionId: versionId, library: library,
                        identity: account.identities[.development]?.data)
                }
            }
            .sheet(isPresented: $signingIn) {
                StoreSignInView()
            }
        }
    }
}

/// Every version the App Store still has, newest first, each to get
struct StoreVersionsView: View {
    @Environment(AccountStore.self) private var account
    let app: CatalogApp
    @State private var ids: [String]?
    @State private var error: String?
    @State private var signingIn = false
    @State private var filter = ""

    private var store: AppStoreModel { .shared }

    var body: some View {
        Group {
            if store.signedInAccount == nil {
                ContentUnavailableView {
                    Label("Sign In to See Versions", systemImage: "person.crop.circle")
                } description: {
                    Text("The App Store lists an app's versions to signed-in accounts.")
                } actions: {
                    Button("Sign In") { signingIn = true }
                }
            } else if let ids {
                let shown = filtered(ids)
                List {
                    Section {
                        // A field in the list, not .searchable: adding a
                        // search field once the versions arrive crashed
                        HStack(spacing: 8) {
                            Image(systemName: "line.3.horizontal.decrease")
                                .foregroundStyle(.secondary)
                            TextField("Filter by Version or Version ID", text: $filter)
                                .autocorrectionDisabled()
                                .textInputAutocapitalization(.never)
                            if !filter.isEmpty {
                                Button {
                                    filter = ""
                                } label: {
                                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                    Section {
                        ForEach(shown, id: \.self) { id in
                            row(id, latest: id == ids.first)
                        }
                    } header: {
                        Text(filter.isEmpty ? "\(ids.count) versions" : "\(shown.count) of \(ids.count) versions")
                            .font(.subheadline.weight(.medium))
                            .textCase(nil)
                    } footer: {
                        Text("As the App Store still has them. Each you get is added to your library beside the others.")
                    }
                }
                .listStyle(.insetGrouped)
                .scrollDismissesKeyboard(.immediately)
            } else if let error {
                ContentUnavailableView {
                    Label("Couldn't List Versions", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(error)
                } actions: {
                    Button("Try Again") { Task { await load() } }
                }
            } else {
                VStack(spacing: 12) {
                    ProgressView()
                    Text(store.readying ?? "Getting the versions")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Version History")
        .navigationBarTitleDisplayMode(.large)
        .sheet(isPresented: $signingIn) { StoreSignInView() }
        .task(id: store.signedInAccount?.email) {
            if store.signedInAccount != nil && ids == nil { await load() }
        }
    }

    /// Those whose ID, or number once known, has what's typed
    private func filtered(_ ids: [String]) -> [String] {
        let filter = filter.trimmingCharacters(in: .whitespaces)
        guard !filter.isEmpty else { return ids }
        return ids.filter { id in
            id.contains(filter) || store.versionInfo[id]?.displayVersion.contains(filter) == true
        }
    }

    private func row(_ id: String, latest: Bool) -> some View {
        let info = store.versionInfo[id]
        return HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    if let info {
                        Text("Version \(info.displayVersion)")
                            .font(.body.weight(.semibold))
                    } else {
                        Text("Version …")
                            .font(.body.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    if latest {
                        Badge("Latest", color: .green)
                    }
                }
                if let date = info.flatMap({ SourceDate.parse($0.releaseDate) }) {
                    Text(date, format: .dateTime.day().month().year())
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Text("Version ID \(id)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
            .contextMenu { CopyMenu(title: "Version ID", value: id) }
            Spacer()
            StoreGetButton(app: app, versionId: latest ? nil : id)
        }
        .padding(.vertical, 4)
        .task { await store.loadVersionInfo(id, of: app) }
    }

    private func load() async {
        error = nil
        do {
            ids = try await store.versions(of: app, identity: account.identities[.development]?.data)
        } catch {
            self.error = error.userMessage
        }
    }
}
