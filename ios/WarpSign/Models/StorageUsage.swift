import Foundation

/// What WarpSign keeps on disk, for Settings → Storage
struct StorageUsage: Sendable {
    var originals: UInt64 = 0
    var signedCopies: UInt64 = 0
    /// Of `signedCopies`: copies with a newer one of the same app and certificate
    var superseded: UInt64 = 0
    var supersededCount = 0
    var caches: UInt64 = 0
    var temporary: UInt64 = 0

    var total: UInt64 { originals + signedCopies + caches + temporary }

    /// Provisioning profiles and capabilities kept between signs; all of
    /// it is fetched again when needed
    nonisolated static var cachesDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Caches", isDirectory: true)
    }

    /// Left in the temporary folder for longer than this, it isn't in use
    nonisolated static let staleAfter: TimeInterval = 15 * 60

    /// The library's files, from `LibraryStore.storageFiles()`
    struct Files: Sendable {
        var originals: [URL] = []
        var signedCopies: [URL] = []
        var superseded: [URL] = []
    }

    /// Off the main thread: it reads every file's size
    nonisolated static func measure(_ files: Files) -> StorageUsage {
        var usage = StorageUsage()
        usage.originals = files.originals.reduce(0) { $0 + size(of: $1) }
        usage.signedCopies = files.signedCopies.reduce(0) { $0 + size(of: $1) }
        usage.superseded = files.superseded.reduce(0) { $0 + size(of: $1) }
        usage.supersededCount = files.superseded.count
        usage.caches = size(of: cachesDirectory)
        usage.temporary = size(of: FileManager.default.temporaryDirectory)
        return usage
    }

    /// Empty the caches; they fill again as needed
    nonisolated static func clearCaches() {
        try? FileManager.default.removeItem(at: cachesDirectory)
        try? FileManager.default.createDirectory(at: cachesDirectory, withIntermediateDirectories: true)
    }

    /// Remove what work left behind in the temporary folder: apps unpacked
    /// for signing or inspecting, downloads. Only what's older than
    /// `staleAfter`, so nothing in use goes.
    nonisolated static func clearTemporary() {
        let fileManager = FileManager.default
        let cutoff = Date().addingTimeInterval(-staleAfter)
        let items = (try? fileManager.contentsOfDirectory(
            at: fileManager.temporaryDirectory,
            includingPropertiesForKeys: [.contentModificationDateKey]
        )) ?? []
        for item in items {
            let modified = (try? item.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, modified < cutoff {
                try? fileManager.removeItem(at: item)
            }
        }
    }

    /// Bytes on disk, for a file or everything under a folder
    nonisolated static func size(of url: URL) -> UInt64 {
        let keys: Set<URLResourceKey> = [.totalFileAllocatedSizeKey, .isRegularFileKey]
        func fileSize(_ url: URL) -> UInt64 {
            guard let values = try? url.resourceValues(forKeys: keys), values.isRegularFile == true else { return 0 }
            return UInt64(values.totalFileAllocatedSize ?? 0)
        }
        guard let enumerator = FileManager.default.enumerator(at: url, includingPropertiesForKeys: Array(keys)) else {
            return fileSize(url)
        }
        var total = fileSize(url)
        for case let item as URL in enumerator {
            total += fileSize(item)
        }
        return total
    }
}
