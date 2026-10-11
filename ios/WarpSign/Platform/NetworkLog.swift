import Foundation

/// The last requests to Apple, for working out what went wrong. Kept in
/// memory only. No headers, so no cookies or tokens; error bodies are kept
/// (Apple's message is usually there), success bodies aren't.
final class NetworkLog: @unchecked Sendable {
    static let shared = NetworkLog()

    struct Entry: Identifiable, Sendable {
        let id = UUID()
        let date: Date
        let method: String
        let url: String
        let status: Int?
        let detail: String?
    }

    private let lock = NSLock()
    private var entries: [Entry] = []
    private let limit = 200

    func record(method: String, url: String, status: Int?, detail: String?) {
        // Drop the query: it can hold identifiers
        let shown = url.split(separator: "?", maxSplits: 1).first.map(String.init) ?? url
        let entry = Entry(date: Date(), method: method, url: shown, status: status, detail: detail)
        lock.lock()
        entries.append(entry)
        if entries.count > limit { entries.removeFirst(entries.count - limit) }
        lock.unlock()
    }

    var snapshot: [Entry] {
        lock.lock()
        defer { lock.unlock() }
        return entries
    }

    func clear() {
        lock.lock()
        entries.removeAll()
        lock.unlock()
    }

    var transcript: String {
        let formatter = ISO8601DateFormatter()
        return snapshot.map { entry in
            var line = "\(formatter.string(from: entry.date)) \(entry.method) \(entry.url) → \(entry.status.map(String.init) ?? "failed")"
            if let detail = entry.detail { line += "\n    \(detail)" }
            return line
        }
        .joined(separator: "\n")
    }
}
