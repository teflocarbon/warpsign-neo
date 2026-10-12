import Foundation
import Observation

/// Updates for apps downloaded from sources. An app has one when its
/// source's newest version is newer than the library's (`isNewerVersion`).
/// Updating downloads it in place of the library's IPA, signs it with the
/// options its last signed copy was made with (and the same icon), and
/// installs it with the pairing file, as WarpSign would by hand. Runs when
/// asked, and overnight while charging by itself (`BackgroundUpdates`).
@MainActor
@Observable
final class Updater {
    static let shared = Updater()

    struct Available: Identifiable, Equatable {
        let appId: UUID
        let sourceName: String
        let version: AppVersion
        var id: UUID { appId }
    }

    enum Status: Equatable {
        case waiting
        case downloading(Double)
        case signing
        case installing
        case done(String)
        case failed(String)
    }

    /// An update made, for the list in Settings
    struct Entry: Codable, Identifiable {
        var id = UUID()
        let appName: String
        let from: String
        let to: String
        let date: Date
        /// Why it stopped short of installed, if it did
        let problem: String?
        /// How it ended when that wasn't a problem (signed, not installed)
        var note: String? = nil
    }

    /// A signed update to install over the web, which iOS asks the user to
    /// confirm: shown by the app as the install screen
    struct PendingInstall: Identifiable {
        let id = UUID()
        let ipa: URL
        let name: String
        let icon: URL
    }

    private(set) var available: [UUID: Available] = [:]
    /// Web installs from updates the user started, shown one at a time
    var pendingInstalls: [PendingInstall] = []
    private(set) var status: [UUID: Status] = [:]
    private(set) var history: [Entry] = []
    private(set) var lastCheck: Date?
    private(set) var isChecking = false
    private(set) var isUpdating = false

    /// Update apps by themselves, overnight while charging
    var automatic: Bool {
        didSet { UserDefaults.standard.set(automatic, forKey: Keys.automatic) }
    }

    // How automatic updates run (updates started by hand ignore these)

    /// Only while the phone is charging; off, whenever iOS likes
    var requireCharging: Bool {
        didSet { UserDefaults.standard.set(requireCharging, forKey: Keys.requireCharging) }
    }
    /// Not over cellular. Low Data Mode always stops them.
    var wifiOnly: Bool {
        didSet { UserDefaults.standard.set(wifiOnly, forKey: Keys.wifiOnly) }
    }
    /// Install once signed; off, they wait in the library to be installed
    var installUpdates: Bool {
        didSet { UserDefaults.standard.set(installUpdates, forKey: Keys.installUpdates) }
    }
    /// Notify when apps were updated, not only when one needs the user
    var notifySuccess: Bool {
        didSet { UserDefaults.standard.set(notifySuccess, forKey: Keys.notifySuccess) }
    }

    // Which updates there are (by hand too)

    /// Ignore a source's pre-releases, unless the library's version is one
    var skipBetas: Bool {
        didSet {
            UserDefaults.standard.set(skipBetas, forKey: Keys.skipBetas)
            evaluate()
        }
    }
    /// Whether apps downloaded from now on update by themselves
    var newAppsUpdate: Bool {
        didSet { UserDefaults.standard.set(newAppsUpdate, forKey: Keys.newAppsUpdate) }
    }

    @ObservationIgnored private var library: LibraryStore?
    @ObservationIgnored private var account: AccountStore?
    @ObservationIgnored private var sources: SourcesStore?
    @ObservationIgnored private var device: DeviceStore?

    private enum Keys {
        static let automatic = "updates.automatic"
        static let history = "updates.history"
        static let lastCheck = "updates.lastCheck"
        static let requireCharging = "updates.requireCharging"
        static let wifiOnly = "updates.wifiOnly"
        static let installUpdates = "updates.install"
        static let notifySuccess = "updates.notifySuccess"
        static let skipBetas = "updates.skipBetas"
        static let newAppsUpdate = "updates.newApps"
    }

    init() {
        let defaults = UserDefaults.standard
        automatic = defaults.object(forKey: Keys.automatic) as? Bool ?? true
        requireCharging = defaults.object(forKey: Keys.requireCharging) as? Bool ?? true
        wifiOnly = defaults.object(forKey: Keys.wifiOnly) as? Bool ?? true
        installUpdates = defaults.object(forKey: Keys.installUpdates) as? Bool ?? true
        notifySuccess = defaults.object(forKey: Keys.notifySuccess) as? Bool ?? false
        skipBetas = defaults.object(forKey: Keys.skipBetas) as? Bool ?? true
        newAppsUpdate = defaults.object(forKey: Keys.newAppsUpdate) as? Bool ?? true
        lastCheck = defaults.object(forKey: Keys.lastCheck) as? Date
        if let data = defaults.data(forKey: Keys.history),
           let saved = try? JSONDecoder().decode([Entry].self, from: data) {
            history = saved
        }
    }

    /// The app's stores, set once at launch (background launches too)
    func attach(library: LibraryStore, account: AccountStore, sources: SourcesStore, device: DeviceStore) {
        self.library = library
        self.account = account
        self.sources = sources
        self.device = device
    }

    // MARK: Checking

    /// Check unless that was done in the last hour
    func checkIfDue() async {
        if let lastCheck, Date().timeIntervalSince(lastCheck) < 60 * 60 { return }
        await check()
    }

    /// Compare every app from a source with its source's newest version
    func check() async {
        guard let library, let sources, !isChecking, !Demo.isOn else { return }
        isChecking = true
        defer { isChecking = false }
        linkEarlierDownloads()
        let urls = Set(library.apps.compactMap(\.sourceURL))
        for url in urls {
            await sources.refresh(url)
        }
        evaluate()
        lastCheck = Date()
        UserDefaults.standard.set(lastCheck, forKey: Keys.lastCheck)
    }

    /// Work out the updates from the sources as last downloaded
    func evaluate() {
        guard let library, let sources else { return }
        var found: [UUID: Available] = [:]
        for app in library.apps {
            guard let match = sourceVersion(of: app, in: sources),
                  match.version.version != app.skippedVersion,
                  isNewerVersion(
                      candidate: match.version.version,
                      candidateBuild: match.version.build,
                      // What the source called the version we have, if known
                      current: app.sourceVersion ?? app.version ?? "",
                      currentBuild: app.sourceVersion != nil ? app.sourceBuild : app.build
                  )
            else { continue }
            found[app.id] = Available(appId: app.id, sourceName: match.source.name, version: match.version)
        }
        available = found
    }

    /// The app's source, and the version of it there that updates would
    /// offer (skipped or not), as last downloaded
    func sourceVersion(of app: LibraryApp) -> (source: Source, version: AppVersion)? {
        guard let sources else { return nil }
        return sourceVersion(of: app, in: sources)
    }

    private func sourceVersion(of app: LibraryApp, in sources: SourcesStore) -> (source: Source, version: AppVersion)? {
        guard let url = app.sourceURL,
              let source = sources.sources[url],
              let sourceApp = source.apps.first(where: { $0.bundleId == (app.sourceAppId ?? app.bundleId) }),
              let version = newest(of: sourceApp.versions, for: app)
        else { return nil }
        return (source, version)
    }

    /// The version to offer: the source's newest (sources list newest
    /// first), or with Skip Betas its newest release, unless the library
    /// already has a pre-release
    private func newest(of versions: [AppVersion], for app: LibraryApp) -> AppVersion? {
        guard skipBetas, !isPreRelease(version: app.sourceVersion ?? app.version ?? "") else { return versions.first }
        return versions.first { !isPreRelease(version: $0.version) }
    }

    /// Don't offer this update; later versions still are
    func skip(_ id: UUID) {
        guard let library, let app = library.app(id), let update = available[id] else { return }
        library.setSkippedVersion(update.version.version, for: app)
        available[id] = nil
        status[id] = nil
    }

    func stopSkipping(_ id: UUID) {
        guard let library, let app = library.app(id) else { return }
        library.setSkippedVersion(nil, for: app)
        evaluate()
    }

    /// Apps downloaded before sources were remembered: matched by source
    /// name and bundle ID
    private func linkEarlierDownloads() {
        guard let library, let sources else { return }
        for app in library.apps where app.sourceURL == nil {
            guard let origin = app.origin,
                  let url = sources.urls.first(where: { url in
                      sources.sources[url]?.name == origin
                          && sources.sources[url]?.apps.contains(where: { $0.bundleId == app.bundleId }) == true
                  })
            else { continue }
            library.link(app, sourceURL: url, sourceAppId: app.bundleId, autoUpdate: newAppsUpdate)
        }
    }

    // MARK: Updating

    /// Update `ids` one after another. Started by the user, the progress is
    /// in the Dynamic Island and each update installs; `automatic` (in the
    /// background) follows the settings above and notifies instead.
    /// Returns what happened.
    @discardableResult
    func run(_ ids: [UUID], automatic: Bool) async -> [Entry] {
        let showCard = !automatic
        guard let library, !isUpdating else { return [] }
        let ids = ids.filter { available[$0] != nil }
        guard !ids.isEmpty else { return [] }
        isUpdating = true
        defer { isUpdating = false }
        for id in ids { status[id] = .waiting }
        if showCard {
            let title = ids.count == 1
                ? "Updating \(library.app(ids[0])?.displayName ?? "an app")"
                : "Updating \(ids.count) apps"
            ContinuedWork.shared.begin(title: title, subtitle: "Starting")
        }
        defer { if showCard { ContinuedWork.shared.finish() } }

        var entries: [Entry] = []
        for (index, id) in ids.enumerated() {
            if showCard, let name = library.app(id)?.displayName {
                ContinuedWork.shared.update(subtitle: name, fraction: Double(index) / Double(ids.count))
            }
            if let entry = await update(id, automatic: automatic) {
                entries.append(entry)
                if automatic, let problem = entry.problem {
                    Notifier.post(title: "\(entry.appName) wasn't updated", body: problem)
                }
            }
        }
        if automatic && notifySuccess {
            notifyUpdated(entries.filter { $0.problem == nil })
        }
        return entries
    }

    /// One notification for everything that updated
    private func notifyUpdated(_ entries: [Entry]) {
        guard !entries.isEmpty else { return }
        let names = entries.map(\.appName)
        let list = names.count <= 3
            ? ListFormatter.localizedString(byJoining: names)
            : "\(names.prefix(2).joined(separator: ", ")) and \(names.count - 2) more"
        let signedOnly = entries.allSatisfy { $0.note != nil }
        Notifier.post(
            title: signedOnly ? "Updates Ready to Install" : "Apps Updated",
            body: signedOnly ? "\(list): signed, and waiting in WarpSign." : "\(list) updated."
        )
    }

    /// Whether automatic updates install, rather than stopping at signed:
    /// installing over the web needs the user to confirm each install
    var installsAutomatically: Bool {
        installUpdates && device?.method == .pairing
    }

    /// Every app with an update that's set to update by itself
    var automaticIds: [UUID] {
        guard let library else { return [] }
        return available.keys.filter { library.app($0)?.updatesAutomatically ?? false }
    }

    private func update(_ id: UUID, automatic: Bool) async -> Entry? {
        guard let library, let account, let device,
              let update = available[id], let app = library.app(id)
        else { return nil }
        let from = app.versionText
        let to = update.version.version
        status[id] = .downloading(0)
        do {
            if app.signedCopies.contains(where: { InstallView.isThisApp($0.bundleId) }) {
                throw WarpsignError.Device(message: "WarpSign updates itself through WarpSign Companion: sign it, then Share Signed IPA → Companion.")
            }
            guard let url = URL(string: update.version.downloadUrl) else {
                throw WarpsignError.Source(message: "The source's download link isn't valid.")
            }
            let file = try await DownloadManager.shared.downloadUpdate(
                url, name: "\(app.displayName) \(to)", appId: id
            ) { fraction in
                if case .downloading = self.status[id] { self.status[id] = .downloading(fraction) }
            }
            defer { try? FileManager.default.removeItem(at: file) }
            var updated = try await library.replaceIPA(of: app, with: file)
            library.setSourceVersion(update.version.version, build: update.version.build, for: updated)
            updated = library.app(id) ?? updated
            available[id] = nil

            guard let options = library.lastSigningOptions(of: updated) else {
                return finish(id, app: updated, from: from, to: to, done: "Downloaded",
                              problem: "Downloaded \(to). Sign it once, and later updates install by themselves.")
            }
            if account.phase == .signedOut { await account.restore() }
            status[id] = .signing
            let signLog = StepLog(signSteps())
            let (_, copy) = try await SignJob.run(
                app: updated,
                options: options,
                icon: library.customIcon(of: updated),
                account: account,
                library: library,
                observer: StepRecorder { event in signLog.steps.apply(event) },
                steps: { signLog.steps }
            )
            // The old version's signed copies: the library no longer has
            // that version, and each is a whole IPA
            library.removeSignedCopies(of: updated, except: copy)
            // Automatic updates install only with the pairing file (iOS asks
            // before each web install), and only with Install Updates on
            if automatic && !installsAutomatically {
                return finish(id, app: updated, from: from, to: to, done: "Signed: ready to install",
                              problem: nil, note: "Signed, not installed")
            }
            guard device.isReady else {
                return finish(id, app: updated, from: from, to: to, done: "Signed",
                              problem: "Signed \(to). Set up installing in Settings, then install it from the Library.")
            }
            if device.method == .web {
                // Started by the user, who's there to confirm iOS's prompt
                pendingInstalls.append(PendingInstall(
                    ipa: library.signedURL(copy, of: updated),
                    name: updated.displayName,
                    icon: LibraryStore.folder(of: updated.id).appendingPathComponent("icon.png")
                ))
                return finish(id, app: updated, from: from, to: to, done: "Signed: confirm the install",
                              problem: nil, note: "Sent to install over the web")
            }
            status[id] = .installing
            let installLog = StepLog(installSteps())
            try await device.install(
                library.signedURL(copy, of: updated),
                observer: StepRecorder { event in installLog.steps.apply(event) }
            )
            return finish(id, app: updated, from: from, to: to, done: "Updated to \(to)", problem: nil)
        } catch {
            status[id] = .failed(error.userMessage)
            return record(Entry(appName: app.displayName, from: from, to: to, date: Date(), problem: error.userMessage))
        }
    }

    private func finish(
        _ id: UUID, app: LibraryApp, from: String, to: String, done: String, problem: String?, note: String? = nil
    ) -> Entry {
        status[id] = .done(done)
        return record(Entry(appName: app.displayName, from: from, to: to, date: Date(), problem: problem, note: note))
    }

    private func record(_ entry: Entry) -> Entry {
        history.insert(entry, at: 0)
        history = Array(history.prefix(50))
        if let data = try? JSONEncoder().encode(history) {
            UserDefaults.standard.set(data, forKey: Keys.history)
        }
        return entry
    }
}

/// Steps reported while nothing shows them, kept for the signing record
@MainActor
final class StepLog {
    var steps: [StepState]

    init(_ steps: [StepInfo]) {
        self.steps = [StepState](pending: steps)
    }
}
