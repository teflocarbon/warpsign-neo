import Foundation
import Observation
import UIKit

/// An IPA in the library
struct LibraryApp: Codable, Identifiable, Hashable, Sendable {
    let id: UUID
    var displayName: String
    var bundleId: String
    var version: String?
    var build: String?
    var minimumOS: String?
    var fileSize: UInt64
    var installedSize: UInt64
    var extensionCount: UInt32
    /// "decrypted", "encrypted" or "unknown"
    var encryption: String
    var addedAt: Date
    /// Where it came from: a source's name, or nil if imported from a file
    var origin: String?
    var signedCopies: [SignedCopy] = []

    var isEncrypted: Bool { encryption == "encrypted" }

    var versionText: String {
        switch (version, build) {
        case let (version?, build?) where build != version: "\(version) (\(build))"
        case let (version?, _): version
        case let (nil, build?): build
        default: "—"
        }
    }
}

/// A signed IPA made from a library app
struct SignedCopy: Codable, Identifiable, Hashable, Sendable {
    let id: UUID
    var date: Date
    var fileName: String
    var bundleId: String
    var teamId: String
    var certificate: String
    var ppqCheck: Bool
    var profileExpires: Date?
}

/// The imported IPAs, kept in Application Support/Library:
/// `<id>/app.ipa`, `<id>/icon.png` and `<id>/signed/<copy id>/<name>.ipa`
@MainActor
@Observable
final class LibraryStore {
    private(set) var apps: [LibraryApp] = []
    /// Files being imported, by name
    private(set) var importing: [String] = []
    var importError: String?

    @ObservationIgnored private var icons: [UUID: UIImage] = [:]

    nonisolated static let directory: URL = {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("Library", isDirectory: true)
    }()

    private static var indexURL: URL { directory.appendingPathComponent("library.json") }

    init() {
        if let data = try? Data(contentsOf: Self.indexURL),
           let apps = try? JSONDecoder().decode([LibraryApp].self, from: data) {
            self.apps = apps
        }
    }

    nonisolated static func folder(of id: UUID) -> URL {
        directory.appendingPathComponent(id.uuidString, isDirectory: true)
    }

    nonisolated static func ipaURL(of id: UUID) -> URL {
        folder(of: id).appendingPathComponent("app.ipa")
    }

    func app(_ id: UUID) -> LibraryApp? {
        apps.first { $0.id == id }
    }

    func icon(for id: UUID) -> UIImage? {
        if let icon = icons[id] { return icon }
        let url = Self.folder(of: id).appendingPathComponent("icon.png")
        // iOS decodes Apple's CgBI PNGs, which apps ship their icons as
        guard let data = try? Data(contentsOf: url), let image = UIImage(data: data) else {
            return nil
        }
        icons[id] = image
        return image
    }

    func signedURL(_ copy: SignedCopy, of app: LibraryApp) -> URL {
        Self.folder(of: app.id)
            .appendingPathComponent("signed", isDirectory: true)
            .appendingPathComponent(copy.id.uuidString, isDirectory: true)
            .appendingPathComponent(copy.fileName)
    }

    /// Copy an IPA (from Files or the share sheet) into the library
    @discardableResult
    func importFile(_ url: URL, origin: String? = nil, move: Bool = false) async -> LibraryApp? {
        let name = url.lastPathComponent
        importing.append(name)
        defer { importing.removeAll { $0 == name } }
        do {
            let app = try await background {
                try Self.add(url, origin: origin, move: move)
            }
            apps.insert(app, at: 0)
            save()
            importError = nil
            return app
        } catch {
            importError = "\(name): \(error.userMessage)"
            return nil
        }
    }

    /// IPAs can be large, and can be downloaded again: keep them out of
    /// iCloud backups
    nonisolated private static func excludeFromBackup() {
        var directory = Self.directory
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? directory.setResourceValues(values)
    }

    nonisolated private static func add(_ url: URL, origin: String?, move: Bool) throws -> LibraryApp {
        let id = UUID()
        let folder = folder(of: id)
        let fileManager = FileManager.default
        try fileManager.createDirectory(at: folder, withIntermediateDirectories: true)
        excludeFromBackup()
        let destination = ipaURL(of: id)
        do {
            try withSecurityScopedAccess(to: url) {
                if move {
                    try fileManager.moveItem(at: url, to: destination)
                } else {
                    try fileManager.copyItem(at: url, to: destination)
                }
            }
            let metadata = try readIpaMetadata(ipaPath: destination.path)
            if let icon = metadata.icon {
                try icon.write(to: folder.appendingPathComponent("icon.png"))
            }
            let encryption = switch metadata.encryption {
            case .decrypted: "decrypted"
            case .encrypted: "encrypted"
            case .unknown: "unknown"
            }
            return LibraryApp(
                id: id,
                displayName: metadata.displayName,
                bundleId: metadata.bundleId,
                version: metadata.version,
                build: metadata.build,
                minimumOS: metadata.minimumOs,
                fileSize: metadata.fileSize,
                installedSize: metadata.installedSize,
                extensionCount: metadata.extensionCount,
                encryption: encryption,
                addedAt: Date(),
                origin: origin
            )
        } catch {
            try? fileManager.removeItem(at: folder)
            throw error
        }
    }

    func delete(_ app: LibraryApp) {
        apps.removeAll { $0.id == app.id }
        icons[app.id] = nil
        try? FileManager.default.removeItem(at: Self.folder(of: app.id))
        save()
    }

    /// Where to write a new signed copy, and its record
    func newSignedCopy(of app: LibraryApp) -> (URL, SignedCopy) {
        let base = app.displayName
            .components(separatedBy: CharacterSet(charactersIn: "/\\:?%*|\"<>"))
            .joined()
        let fileName = "\(base.isEmpty ? "App" : base) \(app.version ?? "") signed.ipa"
            .replacingOccurrences(of: "  ", with: " ")
        let copy = SignedCopy(
            id: UUID(),
            date: Date(),
            fileName: fileName,
            bundleId: "",
            teamId: "",
            certificate: "",
            ppqCheck: false
        )
        let url = signedURL(copy, of: app)
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        return (url, copy)
    }

    func addSignedCopy(_ copy: SignedCopy, to app: LibraryApp) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        apps[index].signedCopies.insert(copy, at: 0)
        save()
    }

    func deleteSignedCopy(_ copy: SignedCopy, of app: LibraryApp) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        apps[index].signedCopies.removeAll { $0.id == copy.id }
        try? FileManager.default.removeItem(at: signedURL(copy, of: app).deletingLastPathComponent())
        save()
    }

    private func save() {
        do {
            try FileManager.default.createDirectory(at: Self.directory, withIntermediateDirectories: true)
            let data = try JSONEncoder().encode(apps)
            try data.write(to: Self.indexURL, options: .atomic)
        } catch {
            importError = "Couldn't save the library: \(error.localizedDescription)"
        }
    }
}
