import Foundation

/// Room on this device, checked before work that needs plenty of it, so it
/// fails at once with what's needed rather than partway through
enum DiskSpace {
    /// Kept free beyond what the work itself needs
    static let margin: Int64 = 200_000_000

    /// What iOS would free up for something the user asked for
    static var available: Int64? {
        let values = try? URL.documentsDirectory.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        return values?.volumeAvailableCapacityForImportantUsage
    }

    /// Throws when there's less than `needed` (plus a margin) free.
    /// `what` names the work: "Downloading YouTube".
    static func ensure(_ needed: Int64, for what: String) throws {
        guard let available, available < needed + margin else { return }
        throw WarpsignError.Device(message: message(needed: needed, available: available, for: what))
    }

    static func message(needed: Int64, available: Int64, for what: String) -> String {
        let need = ByteCountFormatter.file(UInt64(max(needed + margin, 0)))
        let free = ByteCountFormatter.file(UInt64(max(available, 0)))
        return "\(what) needs about \(need) free, and this iPhone has \(free). Free some space (Settings → Storage in WarpSign can help), then try again."
    }
}
