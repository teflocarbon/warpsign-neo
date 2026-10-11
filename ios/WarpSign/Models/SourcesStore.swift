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
    /// Download URL → progress (0...1)
    private(set) var downloads: [String: Double] = [:]
    var downloadError: String?

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

    /// Add a source after checking it loads
    func add(_ text: String) async throws {
        var text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.contains("://") { text = "https://" + text }
        guard let url = URL(string: text), url.scheme == "https" || url.scheme == "http" else {
            throw WarpsignError.Source(message: "That isn't a web address.")
        }
        guard !urls.contains(text) else { return }
        let source = try await Self.fetch(url)
        urls.append(text)
        sources[text] = source
        save()
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

    /// Download an app version and add it to the library
    func download(_ version: AppVersion, of app: SourceApp, from source: Source, into library: LibraryStore) async {
        let key = version.downloadUrl
        guard downloads[key] == nil, let url = URL(string: key) else { return }
        downloads[key] = 0
        defer { downloads[key] = nil }
        do {
            let file = try await Downloader.download(url) { progress in
                Task { @MainActor in
                    if self.downloads[key] != nil { self.downloads[key] = progress }
                }
            }
            await library.importFile(file, origin: source.name, move: true)
            try? FileManager.default.removeItem(at: file.deletingLastPathComponent())
            downloadError = nil
        } catch {
            downloadError = "\(app.name): \(error.userMessage)"
        }
    }

    private func save() {
        guard !isDemo else { return }
        UserDefaults.standard.set(urls, forKey: Self.key)
    }
}

/// A download with progress, kept as a file of our own
enum Downloader {
    static func download(
        _ url: URL,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> URL {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let name = url.pathExtension.lowercased() == "ipa" ? url.lastPathComponent : "Download.ipa"
        let destination = folder.appendingPathComponent(name.removingPercentEncoding ?? name)

        let observation = ObservationHolder()
        return try await withCheckedThrowingContinuation { continuation in
            let task = URLSession.shared.downloadTask(with: url) { file, response, error in
                defer { observation.value = nil }
                if let error {
                    continuation.resume(throwing: error)
                    return
                }
                if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
                    continuation.resume(throwing: WarpsignError.Source(message: "The download failed with HTTP \(http.statusCode)."))
                    return
                }
                guard let file else {
                    continuation.resume(throwing: WarpsignError.Source(message: "The download is empty."))
                    return
                }
                do {
                    // The temporary file is deleted when this returns
                    try FileManager.default.moveItem(at: file, to: destination)
                    continuation.resume(returning: destination)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
            observation.value = task.progress.observe(\.fractionCompleted) { value, _ in
                progress(value.fractionCompleted)
            }
            task.resume()
        }
    }

    /// Keeps the progress observation alive until the download finishes
    private final class ObservationHolder: @unchecked Sendable {
        var value: NSKeyValueObservation?
    }
}
