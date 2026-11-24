import Foundation
import Observation

/// The App Store tab's state: the Apple ID ipatool is signed in to, getting
/// Unicorn and ipatool's runtime ready, the account's apps, and downloads,
/// which go into the library.
///
/// Browsing (search, app pages) uses the public catalog (AppStoreCatalog)
/// and needs none of this. Signing in, listing apps and versions, and
/// downloading run Apple's code in Unicorn, so each first makes sure it's
/// ready (downloaded once, then signed and loaded each launch).
@Observable
@MainActor
final class AppStoreModel {
    static let shared = AppStoreModel()

    enum AccountState: Equatable {
        case unknown
        case signedOut
        case signedIn(StoreAccountInfo)
    }

    var account: AccountState = .unknown
    /// What getting ready is doing ("Signing Unicorn"), while it is
    var readying: String?
    /// The apps the account has got, newest first, once listed
    var owned: [CatalogApp]?
    var ownedTotal = 0
    var downloads: [Int64: StoreDownload] = [:]
    /// Each version's number and date, by external version ID
    var versionInfo: [String: StoreVersionInfo] = [:]

    /// ipatool's runtime is loaded for this launch
    private var prepared = false
    /// The last version lookup asked for, which the next waits on
    private var versionLookups: Task<Void, Never>?

    private init() {}

    var signedInAccount: StoreAccountInfo? {
        if case .signedIn(let info) = account { info } else { nil }
    }

    /// The account's store's country, or the device's when signed out
    var accountCountry: String {
        signedInAccount.flatMap { AppStoreCatalog.country(storefront: $0.storefront) }
            ?? AppStoreCatalog.deviceCountry
    }

    /// Another country's store to browse, chosen in the tab
    var chosenCountry: String? = UserDefaults.standard.string(forKey: Keys.country) {
        didSet { UserDefaults.standard.set(chosenCountry, forKey: Keys.country) }
    }

    /// The country whose store to browse: the one chosen, or the account's
    var country: String { chosenCountry ?? accountCountry }

    /// Searches made, newest first
    private(set) var recentSearches: [String] = UserDefaults.standard.stringArray(forKey: Keys.recent) ?? [] {
        didSet { UserDefaults.standard.set(recentSearches, forKey: Keys.recent) }
    }

    /// The charts, by country and chart, once fetched this launch
    private(set) var charts: [String: [CatalogApp]] = [:]

    private enum Keys {
        static let country = "AppStoreCountry"
        static let recent = "AppStoreRecentSearches"
    }

    /// Unicorn is loaded and runs, and ipatool's runtime too, this launch
    var isReady: Bool { UnicornRuntime.isReady && prepared }

    // MARK: Account

    /// Which account ipatool has (it keeps the session itself)
    func refreshAccount() async {
        guard AppStoreEngine.isAvailable else {
            account = .signedOut
            return
        }
        let info = try? await background { try AppStoreEngine.client().account() }
        account = info.map(AccountState.signedIn) ?? .signedOut
    }

    /// Throws StoreCodeRequired when Apple wants the two-factor code it has
    /// just sent: ask for it, and sign in again with it
    func signIn(email: String, password: String, code: String?, identity: IdentityData?) async throws {
        try await ready(identity: identity)
        let info = try await background {
            try AppStoreEngine.client().login(email: email, password: password, code: code)
        }
        account = .signedIn(info)
        owned = nil
    }

    func signOut() async {
        _ = try? await background { try AppStoreEngine.client().logout() }
        account = .signedOut
        owned = nil
        ownedTotal = 0
    }

    // MARK: Browsing

    func remember(search: String) {
        recentSearches.removeAll { $0.caseInsensitiveCompare(search) == .orderedSame }
        recentSearches.insert(search, at: 0)
        recentSearches = Array(recentSearches.prefix(10))
    }

    func forgetSearches(at offsets: IndexSet) {
        recentSearches.remove(atOffsets: offsets)
    }

    func clearRecentSearches() {
        recentSearches = []
    }

    func chart(_ chart: AppStoreCatalog.Chart) -> [CatalogApp]? {
        charts["\(country)/\(chart.rawValue)"]
    }

    /// Fetch the chart for the store being browsed (again, with `refresh`)
    func loadChart(_ chart: AppStoreCatalog.Chart, refresh: Bool = false) async throws {
        let key = "\(country)/\(chart.rawValue)"
        guard refresh || charts[key] == nil else { return }
        charts[key] = try await AppStoreCatalog.chart(chart, country: country)
    }

    /// Search the store being browsed. An app named by link, ID or bundle
    /// ID that it hasn't got is looked for in the account's store and the
    /// US one; `foundIn` says where it was.
    func find(_ text: String) async throws -> (apps: [CatalogApp], foundIn: String?) {
        try await AppStoreCatalog.find(text, country: country, fallbacks: [accountCountry, "US"])
    }

    // MARK: Getting ready

    /// Unicorn, then ipatool's runtime and Apple's assets (fetched
    /// once, about 190 MB). Skips whatever's done.
    func ready(identity: IdentityData?) async throws {
        defer { readying = nil }
        if !UnicornRuntime.isReady {
            guard let identity else { throw UnicornRuntime.Failure.noCertificate }
            try await UnicornRuntime.prepare(identity: identity) { [weak self] step in
                self?.readying = step
            }
        }
        if !prepared {
            readying = "Getting the App Store ready"
            try await background { try AppStoreEngine.client().prepare() }
            prepared = true
        }
    }

    // MARK: The account's apps

    func loadOwned(identity: IdentityData?) async throws {
        try await ready(identity: identity)
        let page = try await background { try AppStoreEngine.client().owned(page: 1, limit: 100) }
        let country = accountCountry
        // Icons and the rest from the catalog; those it doesn't list keep
        // what ipatool knows
        let listed = (try? await AppStoreCatalog.lookup(ids: page.apps.map(\.id), country: country)) ?? []
        let byId = Dictionary(listed.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        owned = page.apps.map { byId[$0.id] ?? CatalogApp($0) }
        ownedTotal = Int(page.total)
    }

    /// Whether the account has got the app: nil when that isn't known
    /// (its apps not listed yet, or more of them than were listed)
    func owns(_ app: CatalogApp) -> Bool? {
        guard let owned else { return nil }
        if owned.contains(where: { $0.id == app.id }) { return true }
        return ownedTotal > owned.count ? nil : false
    }

    // MARK: Versions

    /// External version IDs, newest first
    func versions(of app: CatalogApp, identity: IdentityData?) async throws -> [String] {
        try await ready(identity: identity)
        let id = String(app.id)
        let ids = try await background { try AppStoreEngine.client().versions(app: id) }
        // Each once: the list shows them by ID
        var seen = Set<String>()
        return ids.reversed().filter { seen.insert($0).inserted }
    }

    /// Fills in versionInfo for one version. Lookups take turns (ipatool
    /// runs one call at a time), in the order asked. One whose row has gone
    /// (cancelled) is skipped when its turn comes: otherwise a long list
    /// scrolled past, or left, holds up everything after it, such as the
    /// next app's versions.
    func loadVersionInfo(_ versionId: String, of app: CatalogApp) async {
        guard versionInfo[versionId] == nil else { return }
        let id = String(app.id)
        let previous = versionLookups
        let lookup = Task {
            await previous?.value
            guard versionInfo[versionId] == nil, !Task.isCancelled else { return }
            if let info = try? await background({
                try AppStoreEngine.client().versionMetadata(app: id, versionId: versionId)
            }) {
                versionInfo[versionId] = info
            }
        }
        versionLookups = lookup
        await withTaskCancellationHandler {
            await lookup.value
        } onCancel: {
            lookup.cancel()
        }
    }

    // MARK: Downloads

    /// Download the app (or one version of it) and add it to the library.
    /// Free apps the account hasn't got are got first, as the App Store's
    /// Get does; paid ones have to be bought in the App Store.
    func download(
        _ app: CatalogApp,
        versionId: String? = nil,
        library: LibraryStore,
        identity: IdentityData?
    ) {
        guard downloads[app.id]?.isActive != true else { return }
        downloads[app.id] = StoreDownload(app: app, versionId: versionId)
        Task {
            do {
                downloads[app.id]?.phase = .preparing
                try await ready(identity: identity)

                let folder = URL.temporaryDirectory
                    .appendingPathComponent("AppStore/\(UUID().uuidString)", isDirectory: true)
                try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                defer { try? FileManager.default.removeItem(at: folder) }

                downloads[app.id]?.phase = .downloading
                // ipatool reports no progress: watch the folder fill
                let watcher = Task {
                    while !Task.isCancelled {
                        let bytes = try? await background { Self.size(of: folder) }
                        self.downloads[app.id]?.received = bytes ?? 0
                        try? await Task.sleep(for: .milliseconds(500))
                    }
                }
                let id = String(app.id), purchase = app.isFree
                let result: StoreDownloadInfo
                do {
                    result = try await background {
                        try AppStoreEngine.client().download(
                            app: id, version: versionId, directory: folder.path, purchase: purchase)
                    }
                } catch {
                    watcher.cancel()
                    throw error
                }
                watcher.cancel()

                downloads[app.id]?.phase = .importing
                let imported = await library.importFile(
                    URL(fileURLWithPath: result.path), origin: "App Store", move: true)
                if imported == nil {
                    throw StoreDownload.Failure(message: library.importError ?? "It couldn't be added to the library.")
                }
                downloads[app.id] = nil
            } catch WarpsignError.StoreNotOwned {
                downloads[app.id]?.phase = .failed(
                    "Your Apple ID doesn't own \(app.name). Buy it in the App Store, then get it here.")
            } catch {
                downloads[app.id]?.phase = .failed(error.userMessage)
            }
        }
    }

    func dismissDownload(_ app: CatalogApp) {
        if downloads[app.id]?.isActive == false {
            downloads[app.id] = nil
        }
    }

    nonisolated private static func size(of folder: URL) -> Int64? {
        guard let files = FileManager.default.enumerator(
            at: folder, includingPropertiesForKeys: [.fileSizeKey]) else { return nil }
        var total: Int64 = 0
        for case let file as URL in files {
            total += Int64((try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
        }
        return total
    }
}

/// An App Store app being downloaded into the library
struct StoreDownload {
    enum Phase: Equatable {
        case waiting
        /// Getting Unicorn and ipatool ready
        case preparing
        case downloading
        case importing
        case failed(String)
    }

    struct Failure: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    let app: CatalogApp
    let versionId: String?
    var phase = Phase.waiting
    var received: Int64 = 0

    var isActive: Bool {
        if case .failed = phase { false } else { true }
    }

    /// How far it's got, when the catalog gave its size. ipatool writes the
    /// download, then a copy with the app's licence added, so the folder
    /// holds up to twice the size: the second half is the last step.
    var fraction: Double? {
        guard phase == .downloading, let size = app.size, size > 0, versionId == nil else { return nil }
        return min(Double(received) / Double(size), 1)
    }

    /// `readying`: what getting ready is doing (AppStoreModel.readying)
    func status(readying: String?) -> String {
        switch phase {
        case .waiting, .preparing: readying ?? "Getting ready"
        case .downloading:
            if let size = app.size, versionId == nil, received > Int64(clamping: size) {
                "Finishing"
            } else {
                "Downloading, \(received.formatted(.byteCount(style: .file)))"
                    + (app.size.map { " of \(Int64(clamping: $0).formatted(.byteCount(style: .file)))" } ?? "")
            }
        case .importing: "Adding to your library"
        case .failed(let message): message
        }
    }
}
