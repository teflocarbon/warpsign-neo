import Foundation
import Observation

/// AltStore-format app sources, and downloads from them into the library
@MainActor
@Observable
final class SourcesStore {
    /// Source URLs, in the order added
    private(set) var urls: [String]
    private(set) var sources: [String: Source] = [:]
    private(set) var errors: [String: String] = [:]
    private(set) var loading: Set<String> = []
    /// A source opened from a link (altstore://, sidestore://, a web page
    /// wrapping one): Sources asks whether to add it
    var proposed: String?

    private static let key = "sources"
    /// Screenshots: sample sources, never fetched
    @ObservationIgnored private var isDemo = false

    init() {
        urls = UserDefaults.standard.stringArray(forKey: Self.key) ?? []
    }

    init(demoSources: [String: Source]) {
        urls = Array(demoSources.keys)
        sources = demoSources
        isDemo = true
    }

    /// Add a source after checking it loads. Takes its address, or a
    /// link that wraps one (altstore://source?url=…, feather://source/…,
    /// or a web page redirecting to either).
    func add(_ text: String) async throws {
        let text = try Self.resolve(text)
        guard let url = URL(string: text) else {
            throw WarpsignError.Source(message: "That isn't a web address.")
        }
        guard !urls.contains(text) else { return }
        let source = try await Self.fetch(url)
        urls.append(text)
        sources[text] = source
        save()
    }

    /// Change a source's address, after checking the new one loads; it
    /// keeps its place. Returns the address it now has.
    @discardableResult
    func replace(_ old: String, with text: String) async throws -> String {
        let new = try Self.resolve(text)
        guard new != old else { return old }
        guard !urls.contains(new) else {
            throw WarpsignError.Source(message: "That source is already added.")
        }
        guard let url = URL(string: new) else {
            throw WarpsignError.Source(message: "That isn't a web address.")
        }
        let source = try await Self.fetch(url)
        guard let index = urls.firstIndex(of: old) else { return old }
        urls[index] = new
        sources[old] = nil
        errors[old] = nil
        sources[new] = source
        save()
        return new
    }

    private static func resolve(_ text: String) throws -> String {
        guard let url = resolveSourceUrl(text: text) else {
            throw WarpsignError.Source(message: "That isn't a web address, or a link to a source.")
        }
        return url
    }

    func remove(at offsets: IndexSet) {
        for index in offsets {
            sources[urls[index]] = nil
            errors[urls[index]] = nil
        }
        urls.remove(atOffsets: offsets)
        save()
    }

    func refresh(_ text: String) async {
        guard !isDemo, let url = URL(string: text), !loading.contains(text) else { return }
        loading.insert(text)
        defer { loading.remove(text) }
        do {
            sources[text] = try await Self.fetch(url)
            errors[text] = nil
        } catch {
            errors[text] = error.userMessage
        }
    }

    func refreshAll() async {
        await withTaskGroup(of: Void.self) { group in
            for url in urls {
                group.addTask { await self.refresh(url) }
            }
        }
    }

    nonisolated private static func fetch(_ url: URL) async throws -> Source {
        var request = URLRequest(url: url)
        request.cachePolicy = .reloadIgnoringLocalCacheData
        let (data, response) = try await URLSession.shared.data(for: request)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw WarpsignError.Source(message: "The source returned HTTP \(http.statusCode).")
        }
        return try await background { try parseSource(data: data) }
    }

    /// Download an app version into the library (`DownloadManager`)
    func download(_ version: AppVersion, of app: SourceApp, from source: Source) {
        guard let url = URL(string: version.downloadUrl) else { return }
        // Remembered, so the library can look here for updates
        let sourceURL = urls.first { sources[$0]?.name == source.name }
        DownloadManager.shared.download(url, name: app.name, purpose: .library(
            origin: source.name,
            sourceURL: sourceURL,
            sourceAppId: app.bundleId,
            autoUpdate: Updater.shared.newAppsUpdate,
            link: nil
        ))
    }

    private func save() {
        guard !isDemo else { return }
        UserDefaults.standard.set(urls, forKey: Self.key)
    }
}
