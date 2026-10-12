import Foundation

/// Signing a library app into a new signed copy, with its record: what the
/// Sign screen does, and what updates do without one
@MainActor
enum SignJob {
    /// `steps` gives the steps as they stand when the record is made
    static func run(
        app: LibraryApp,
        options: SigningOptions,
        icon: Data?,
        account: AccountStore,
        library: LibraryStore,
        observer: StepRecorder,
        steps: @escaping @MainActor () -> [StepState]
    ) async throws -> (SignResult, SignedCopy) {
        // Unpacking it and writing the signed copy: room for both, or say
        // so before starting
        try DiskSpace.ensure(Int64(clamping: app.installedSize &+ app.fileSize), for: "Signing \(app.displayName)")
        let (output, draft) = library.newSignedCopy(of: app)
        do {
            let result = try await account.sign(
                ipa: LibraryStore.ipaURL(of: app.id),
                output: output,
                kind: options.certificate.type,
                settings: options.settings(icon: icon),
                observer: observer
            )
            var copy = draft
            copy.bundleId = result.bundleId
            copy.teamId = account.selectedTeam?.teamId ?? ""
            copy.certificate = options.certificate == .development ? "Development" : "Ad Hoc"
            copy.certificateSerial = account.info(options.certificate.type)?.serialNumber
            copy.ppqCheck = result.profiles.contains { $0.ppqCheck }
            copy.profileExpires = result.profiles
                .compactMap { $0.expiresAt.map(Date.init(unixTime:)) }
                .min()
            // Let the last step events (queued on the main thread) land first
            await Task.yield()
            let record = SigningRecord(result: result, options: options, customIcon: icon != nil, steps: steps())
            library.addSignedCopy(copy, record: record, to: app)
            // Updates sign again with the same icon
            library.setCustomIcon(icon, of: app)
            return (result, copy)
        } catch {
            try? FileManager.default.removeItem(at: output.deletingLastPathComponent())
            throw error
        }
    }
}
