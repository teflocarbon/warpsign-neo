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
        static let ownedIds = "AppStoreOwnedIds"
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
        if signedInAccount?.email != info.email { forgetOwned() }
        account = .signedIn(info)
    }

    func signOut() async {
        _ = try? await background { try AppStoreEngine.client().logout() }
        account = .signedOut
        forgetOwned()
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

    /// List the account's apps: all of them, a page at a time (ipatool gives
    /// 100 at most, fetching the whole list from Apple for each page). One
    /// listing at a time: asking while one runs waits for it.
    func loadOwned(identity: IdentityData?) async throws {
        if let ownedLoad {
            return try await ownedLoad.value
        }
        let load = Task { try await listOwned(identity: identity) }
        ownedLoad = load
        defer { ownedLoad = nil }
        try await load.value
    }

    /// The listing running, which others wait on
    @ObservationIgnored private var ownedLoad: Task<Void, Error>?

    private func listOwned(identity: IdentityData?) async throws {
        try await ready(identity: identity)
        guard let email = signedInAccount?.email else { return }
        let country = accountCountry
        var apps: [CatalogApp] = []
        var ids = Set<Int64>()
        var page: UInt32 = 1
        while true {
            let number = page
            let result = try await background { try AppStoreEngine.client().owned(page: number, limit: 100) }
            // Icons and the rest from the catalog; those it doesn't list
            // keep what ipatool knows
            let listed = (try? await AppStoreCatalog.lookup(ids: result.apps.map(\.id), country: country)) ?? []
            let byId = Dictionary(listed.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            apps += result.apps.map { byId[$0.id] ?? CatalogApp($0) }
            ids.formUnion(result.apps.map(\.id))
            // Shown as each page comes
            guard signedInAccount?.email == email else { return }
            owned = apps
            ownedTotal = Int(result.total)
            if result.apps.isEmpty || apps.count >= Int(result.total) || page >= 100 { break }
            page += 1
        }
        ownedIds = ids
        ownedIdsComplete = true
        Self.saveOwnedIds(ids, account: email)
    }

    /// The App Store IDs of every app the account has got, as last listed
    /// (kept between launches, for this account), so Get can show on paid
    /// apps it has bought before they're listed again
    private(set) var ownedIds: Set<Int64>?
    /// ownedIds is the account's whole list (listed this launch, or kept)
    private var ownedIdsComplete = false

    /// Whether the account has got the app: nil when that isn't known
    /// (its apps not listed yet)
    func owns(_ app: CatalogApp) -> Bool? {
        if ownedIds?.contains(app.id) == true || owned?.contains(where: { $0.id == app.id }) == true {
            return true
        }
        return ownedIdsComplete ? false : nil
    }

    /// List the account's apps once a launch, quietly, so paid apps it has
    /// bought show Get. Starts ipatool (and Unicorn) when there's a
    /// development certificate to sign Unicorn with.
    func refreshOwnership(identity: IdentityData?) async {
        guard let email = signedInAccount?.email, identity != nil || UnicornRuntime.isReady,
              !ownershipChecked else { return }
        ownershipChecked = true
        if ownedIds == nil, let kept = Self.keptOwnedIds(account: email) {
            ownedIds = kept
            ownedIdsComplete = true
        }
        if (try? await loadOwned(identity: identity)) == nil {
            // Try again next time the tab shows
            ownershipChecked = false
        }
    }

    @ObservationIgnored private var ownershipChecked = false

    private static func saveOwnedIds(_ ids: Set<Int64>, account: String) {
        UserDefaults.standard.set(
            ["account": account, "ids": ids.sorted().map(String.init)] as [String: Any],
            forKey: Keys.ownedIds)
    }

    private static func keptOwnedIds(account: String) -> Set<Int64>? {
        guard let kept = UserDefaults.standard.dictionary(forKey: Keys.ownedIds),
              kept["account"] as? String == account,
              let ids = kept["ids"] as? [String] else { return nil }
        return Set(ids.compactMap(Int64.init))
    }

    /// Forget the account's apps (signed out, or another account)
    private func forgetOwned() {
        owned = nil
        ownedTotal = 0
        ownedIds = nil
        ownedIdsComplete = false
        ownershipChecked = false
        UserDefaults.standard.removeObject(forKey: Keys.ownedIds)
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
