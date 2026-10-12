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
    /// The source it was downloaded from, and its bundle ID there, to look
    /// for updates
    var sourceURL: String? = nil
    var sourceAppId: String? = nil
    /// Update by itself when its source has a newer version (nil: yes)
    var autoUpdate: Bool? = nil
    /// A version of it the user chose to skip (a broken release, say): not
    /// offered as an update, though later versions are
    var skippedVersion: String? = nil

    var updatesAutomatically: Bool { autoUpdate ?? true }

    /// Where updates for it can come from
    enum UpdateOrigin: Equatable {
        /// A source in Sources (its URL)
        case source(String)
        /// Downloaded from a source (its name) WarpSign hasn't matched to
        /// one in Sources: removed since, or not checked yet
        case unlinked(String)
        /// WarpSign Companion, which updates with WarpSign
        case bundled
        /// Imported from a file: nowhere to look for updates
        case imported
    }

    var updateOrigin: UpdateOrigin {
        if let sourceURL { return .source(sourceURL) }
        guard let origin else { return .imported }
        return origin == BundledCompanion.origin ? .bundled : .unlinked(origin)
    }

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
    /// The signing certificate's serial (uppercase hex), so moving to a
    /// new certificate knows which apps to sign again. nil for copies
    /// signed before it was recorded.
    var certificateSerial: String?
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
    /// Signing records read so far, by signed copy
    @ObservationIgnored private var records: [UUID: SigningRecord] = [:]
    /// Signed copies with no record (signed before records were kept)
    @ObservationIgnored private var missingRecords: Set<UUID> = []
    /// Screenshots: sample apps, nothing read or written on disk
    @ObservationIgnored private var isDemo = false

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

    init(demoApps: [LibraryApp], icons: [UUID: UIImage], records: [UUID: SigningRecord] = [:]) {
        apps = demoApps
        self.icons = icons
        self.records = records
        isDemo = true
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

    /// Copy an IPA (from Files or the share sheet), or the IPA in a zip, into
    /// the library
    @discardableResult
    func importFile(
        _ url: URL,
        origin: String? = nil,
        move: Bool = false,
        sourceURL: String? = nil,
        sourceAppId: String? = nil,
        autoUpdate: Bool? = nil
    ) async -> LibraryApp? {
        let name = url.lastPathComponent
        guard ["ipa", "zip"].contains(url.pathExtension.lowercased()) else {
            importError = "\(name) isn't an app: choose an .ipa file (or a .zip with one inside)."
            return nil
        }
        importing.append(name)
        defer { importing.removeAll { $0 == name } }
        do {
            var app = try await background {
                try Self.add(url, origin: origin, move: move)
            }
            app.sourceURL = sourceURL
            app.sourceAppId = sourceAppId
            app.autoUpdate = autoUpdate
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
            // A zip with the IPA inside, e.g. a GitHub artifact: keep the IPA
            let unwrapped = folder.appendingPathComponent("unwrapped.ipa")
            if try unwrapIpa(path: destination.path, output: unwrapped.path) {
                try fileManager.removeItem(at: destination)
                try fileManager.moveItem(at: unwrapped, to: destination)
            }
            let metadata = try readIpaMetadata(ipaPath: destination.path)
            if let icon = metadata.icon {
                try icon.write(to: folder.appendingPathComponent("icon.png"))
            }
            let encryption = encryptionName(metadata.encryption)
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

    nonisolated private static func encryptionName(_ encryption: Encryption) -> String {
        switch encryption {
        case .decrypted: "decrypted"
        case .encrypted: "encrypted"
        case .unknown: "unknown"
        }
    }

    // MARK: Updates

    /// Put a newer version of `app` in place of its IPA: the same library
    /// entry, with its signed copies and settings kept. Refuses a different
    /// app.
    func replaceIPA(of app: LibraryApp, with file: URL) async throws -> LibraryApp {
        let id = app.id
        let bundleId = app.bundleId
        let metadata = try await background { () throws -> IpaMetadata in
            let fileManager = FileManager.default
            let folder = Self.folder(of: id)
            let staged = folder.appendingPathComponent("update.ipa")
            try? fileManager.removeItem(at: staged)
            try fileManager.moveItem(at: file, to: staged)
            let unwrapped = folder.appendingPathComponent("unwrapped.ipa")
            if try unwrapIpa(path: staged.path, output: unwrapped.path) {
                try fileManager.removeItem(at: staged)
                try fileManager.moveItem(at: unwrapped, to: staged)
            }
            let metadata = try readIpaMetadata(ipaPath: staged.path)
            guard metadata.bundleId == bundleId else {
                try? fileManager.removeItem(at: staged)
                throw WarpsignError.Ipa(message: "The update is a different app (\(metadata.bundleId)), not \(bundleId).")
            }
            _ = try fileManager.replaceItemAt(Self.ipaURL(of: id), withItemAt: staged)
            if let icon = metadata.icon {
                try? icon.write(to: folder.appendingPathComponent("icon.png"))
            }
            return metadata
        }
        guard let index = apps.firstIndex(where: { $0.id == id }) else {
            throw WarpsignError.Ipa(message: "\(app.displayName) is no longer in the library.")
        }
        apps[index].displayName = metadata.displayName
        apps[index].version = metadata.version
        apps[index].build = metadata.build
        apps[index].minimumOS = metadata.minimumOs
        apps[index].fileSize = metadata.fileSize
        apps[index].installedSize = metadata.installedSize
        apps[index].extensionCount = metadata.extensionCount
        apps[index].encryption = Self.encryptionName(metadata.encryption)
        icons[id] = nil
        save()
        return apps[index]
    }

    func setAutoUpdate(_ on: Bool, for app: LibraryApp) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        apps[index].autoUpdate = on
        save()
    }

    func setSkippedVersion(_ version: String?, for app: LibraryApp) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        apps[index].skippedVersion = version
        save()
    }

    /// Remember which source an app came from (apps downloaded before
    /// sources were remembered are matched by source name and bundle ID)
    func link(_ app: LibraryApp, sourceURL: String, sourceAppId: String, autoUpdate: Bool) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        apps[index].sourceURL = sourceURL
        apps[index].sourceAppId = sourceAppId
        if apps[index].autoUpdate == nil { apps[index].autoUpdate = autoUpdate }
        save()
    }

    /// The icon chosen at the last sign, if any, so updates keep it
    nonisolated static func customIconURL(of id: UUID) -> URL {
        folder(of: id).appendingPathComponent("custom-icon.png")
    }

    func customIcon(of app: LibraryApp) -> Data? {
        try? Data(contentsOf: Self.customIconURL(of: app.id))
    }

    func setCustomIcon(_ icon: Data?, of app: LibraryApp) {
        guard !isDemo else { return }
        let url = Self.customIconURL(of: app.id)
        if let icon {
            try? icon.write(to: url, options: .atomic)
        } else {
            try? FileManager.default.removeItem(at: url)
        }
    }

    /// The options `app` was last signed with, if WarpSign kept them
    func lastSigningOptions(of app: LibraryApp) -> SigningOptions? {
        guard let newest = app.signedCopies.max(by: { $0.date < $1.date }) else { return nil }
        return signingRecord(for: newest, of: app)?.options
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

    /// How a signed copy was made, kept beside its IPA as signing.json
    private func recordURL(_ copy: SignedCopy, of app: LibraryApp) -> URL {
        signedURL(copy, of: app)
            .deletingLastPathComponent()
            .appendingPathComponent("signing.json")
    }

    /// How a signed copy was made: options, entitlements per component and
    /// the steps. nil for copies signed before WarpSign kept these.
    func signingRecord(for copy: SignedCopy, of app: LibraryApp) -> SigningRecord? {
        if let record = records[copy.id] { return record }
        guard !isDemo, !missingRecords.contains(copy.id),
              let data = try? Data(contentsOf: recordURL(copy, of: app)),
              let record = try? JSONDecoder().decode(SigningRecord.self, from: data)
        else {
            missingRecords.insert(copy.id)
            return nil
        }
        records[copy.id] = record
        return record
    }

    func addSignedCopy(_ copy: SignedCopy, record: SigningRecord? = nil, to app: LibraryApp) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        if let record {
            records[copy.id] = record
            if !isDemo, let data = try? JSONEncoder().encode(record) {
                try? data.write(to: recordURL(copy, of: app), options: .atomic)
            }
        }
        apps[index].signedCopies.insert(copy, at: 0)
        save()
    }

    func deleteSignedCopy(_ copy: SignedCopy, of app: LibraryApp) {
        guard let index = apps.firstIndex(where: { $0.id == app.id }) else { return }
        apps[index].signedCopies.removeAll { $0.id == copy.id }
        records[copy.id] = nil
        try? FileManager.default.removeItem(at: signedURL(copy, of: app).deletingLastPathComponent())
        save()
    }

    /// Remove every signed copy of `app` but `keep`: what an update does
    /// once the new version is signed, as the old ones are of a version
    /// the library no longer has
    func removeSignedCopies(of app: LibraryApp, except keep: SignedCopy) {
        guard let app = self.app(app.id) else { return }
        for copy in app.signedCopies where copy.id != keep.id {
            deleteSignedCopy(copy, of: app)
        }
    }

    /// Signed copies with a newer one of the same app and certificate,
    /// which Settings → Storage can remove
    var supersededCopies: [(app: LibraryApp, copy: SignedCopy)] {
        apps.flatMap { app in
            var seen: Set<String> = []
            return app.signedCopies
                .sorted { $0.date > $1.date }
                .filter { !seen.insert($0.certificate).inserted }
                .map { (app: app, copy: $0) }
        }
    }

    func removeSupersededCopies() {
        for (app, copy) in supersededCopies {
            deleteSignedCopy(copy, of: app)
        }
    }

    /// Apps whose newest signed copy used the certificate `serial`; for
    /// copies signed before serials were kept, the same kind of
    /// certificate (`certificate`, e.g. "Ad Hoc") before `before`
    func apps(signedWith serial: String, certificate: String, before: Date) -> [LibraryApp] {
        apps.filter { app in
            guard let newest = app.signedCopies.max(by: { $0.date < $1.date }) else { return false }
            if let used = newest.certificateSerial {
                return used.caseInsensitiveCompare(serial) == .orderedSame
            }
            return newest.certificate == certificate && newest.date < before
        }
    }

    /// Where the library's files are, to measure
    func storageFiles() -> StorageUsage.Files {
        let superseded = Set(supersededCopies.map(\.copy.id))
        var files = StorageUsage.Files()
        for app in apps {
            files.originals.append(Self.ipaURL(of: app.id))
            for copy in app.signedCopies {
                let folder = signedURL(copy, of: app).deletingLastPathComponent()
                files.signedCopies.append(folder)
                if superseded.contains(copy.id) { files.superseded.append(folder) }
            }
        }
        return files
    }

    private func save() {
        guard !isDemo else { return }
        do {
            try FileManager.default.createDirectory(at: Self.directory, withIntermediateDirectories: true)
            let data = try JSONEncoder().encode(apps)
            try data.write(to: Self.indexURL, options: .atomic)
        } catch {
            importError = "Couldn't save the library: \(error.localizedDescription)"
        }
    }
}
