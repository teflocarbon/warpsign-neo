import Foundation
import Observation

/// Keys for decrypting App Store apps right here, with no helper and no
/// device (crates/warpsign-core/src/fairplay.rs): one per Apple ID, 16
/// bytes, from fairplay-decrypt on a jailbroken device or UnfairPlay on a Mac
/// signed in to the App Store. A key decrypts every app its Apple ID got.
///
/// The keys stay in this device's Keychain; only their names and when they
/// were added are kept alongside.
@Observable
@MainActor
final class DecryptionKeys {
    static let shared = DecryptionKeys()

    struct Key: Codable, Identifiable, Hashable {
        let id: UUID
        var name: String
        let added: Date
    }

    private(set) var keys: [Key] = []

    /// Where decrypted apps say they're from in the library
    static let origin = "Decrypted with Key"
    private static let listKey = "DecryptionKeys"

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.listKey),
           let keys = try? JSONDecoder().decode([Key].self, from: data) {
            // A key whose Keychain item is gone (restored to a new device)
            // can't decrypt anything
            self.keys = keys.filter { Keychain.data(for: Self.keychainName($0.id)) != nil }
        }
    }

    private static func keychainName(_ id: UUID) -> String {
        "decrypt-key.\(id.uuidString)"
    }

    private func save() {
        if let data = try? JSONEncoder().encode(keys) {
            UserDefaults.standard.set(data, forKey: Self.listKey)
        }
    }

    func add(name: String, key: Data) throws {
        guard key.count == 16 else {
            throw WarpsignError.Ipa(message: "A key is 16 bytes: that's \(key.count).")
        }
        if let same = keys.first(where: { Keychain.data(for: Self.keychainName($0.id)) == key }) {
            throw WarpsignError.Ipa(message: "That key is already here, as \(same.name).")
        }
        let entry = Key(id: UUID(), name: name, added: Date())
        try Keychain.set(key, for: Self.keychainName(entry.id))
        keys.append(entry)
        save()
    }

    func remove(_ key: Key) {
        try? Keychain.set(nil, for: Self.keychainName(key.id))
        keys.removeAll { $0.id == key.id }
        save()
    }

    func rename(_ key: Key, to name: String) {
        guard let index = keys.firstIndex(where: { $0.id == key.id }) else { return }
        keys[index].name = name
        save()
    }

    /// A key as it's usually written: 32 hex digits (spaces, colons and a
    /// leading 0x allowed) or base64. Nil if it isn't 16 bytes.
    nonisolated static func parse(_ text: String) -> Data? {
        var hex = text.lowercased().filter { !" :\n\r\t-".contains($0) }
        if hex.hasPrefix("0x") { hex.removeFirst(2) }
        if hex.count == 32, hex.allSatisfy(\.isHexDigit) {
            var bytes = [UInt8]()
            var index = hex.startIndex
            while index < hex.endIndex {
                let next = hex.index(index, offsetBy: 2)
                guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
                bytes.append(byte)
                index = next
            }
            return Data(bytes)
        }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if let data = Data(base64Encoded: trimmed), data.count == 16 { return data }
        return nil
    }

    /// A key file: the 16 bytes themselves, or the key written out as text
    nonisolated static func parse(file data: Data) -> Data? {
        if data.count == 16 { return data }
        guard data.count < 256, let text = String(data: data, encoding: .utf8) else { return nil }
        return parse(text)
    }

    /// Decrypt `app` with whichever key fits, and add the decrypted copy to
    /// the library. Returns the name of the key that fitted.
    func decrypt(_ app: LibraryApp, library: LibraryStore, observer: StepRecorder) async throws -> String {
        let entries = keys.compactMap { key in
            Keychain.data(for: Self.keychainName(key.id)).map { (key, $0) }
        }
        guard !entries.isEmpty else {
            throw WarpsignError.Ipa(message: "Add a key first.")
        }
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("KeyDecrypt-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        try DiskSpace.ensure(Int64(clamping: app.fileSize), for: "Decrypting \(app.displayName)")

        let original = LibraryStore.ipaURL(of: app.id)
        let output = folder.appendingPathComponent("\(app.displayName).ipa")
        let credentials = entries.map(\.1)
        let summary = try await background {
            try decryptIpaWithKeys(
                ipaPath: original.path,
                outputPath: output.path,
                credentials: credentials,
                observer: observer
            )
        }
        guard await library.importFile(output, origin: Self.origin, move: true) != nil else {
            throw WarpsignError.Ipa(message: library.importError ?? "The decrypted copy couldn't be added to the library.")
        }
        let index = Int(summary.keyIndex)
        return entries.indices.contains(index) ? entries[index].0.name : "your key"
    }
}
