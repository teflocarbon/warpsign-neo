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
    }

    private(set) var available: [UUID: Available] = [:]
    private(set) var status: [UUID: Status] = [:]
    private(set) var history: [Entry] = []
    private(set) var lastCheck: Date?
    private(set) var isChecking = false
    private(set) var isUpdating = false

    /// Update apps by themselves, overnight while charging
    var automatic: Bool {
        didSet { UserDefaults.standard.set(automatic, forKey: Keys.automatic) }
    }

    @ObservationIgnored private var library: LibraryStore?
    @ObservationIgnored private var account: AccountStore?
    @ObservationIgnored private var sources: SourcesStore?
    @ObservationIgnored private var device: DeviceStore?

    private enum Keys {
        static let automatic = "updates.automatic"
        static let history = "updates.history"
        static let lastCheck = "updates.lastCheck"
    }

    init() {
        let defaults = UserDefaults.standard
        automatic = defaults.object(forKey: Keys.automatic) as? Bool ?? true
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
        var found: [UUID: Available] = [:]
        for app in library.apps {
            guard let url = app.sourceURL,
                  let source = sources.sources[url],
                  let sourceApp = source.apps.first(where: { $0.bundleId == (app.sourceAppId ?? app.bundleId) }),
                  let latest = sourceApp.versions.first,
                  isNewerVersion(
                      candidate: latest.version,
                      candidateBuild: latest.build,
                      current: app.version ?? "",
                      currentBuild: app.build
                  )
            else { continue }
            found[app.id] = Available(appId: app.id, sourceName: source.name, version: latest)
        }
        available = found
        lastCheck = Date()
        UserDefaults.standard.set(lastCheck, forKey: Keys.lastCheck)
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
            library.link(app, sourceURL: url, sourceAppId: app.bundleId)
        }
    }

    // MARK: Updating

    /// Update `ids` one after another. `showCard` puts the progress in the
    /// Dynamic Island (only when the user started it); `notify` reports
    /// what needs the user (background runs). Returns what happened.
    @discardableResult
    func run(_ ids: [UUID], showCard: Bool, notify: Bool) async -> [Entry] {
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
            if let entry = await update(id) {
                entries.append(entry)
                if notify, let problem = entry.problem {
                    Notifier.post(title: "\(entry.appName) wasn't updated", body: problem)
                }
            }
        }
        return entries
    }

    /// Every app with an update that's set to update by itself
    var automaticIds: [UUID] {
        guard let library else { return [] }
        return available.keys.filter { library.app($0)?.updatesAutomatically ?? false }
    }

    private func update(_ id: UUID) async -> Entry? {
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
            let file = try await Downloader.download(url) { fraction in
                Task { @MainActor in
                    if case .downloading = self.status[id] { self.status[id] = .downloading(fraction) }
                }
            }
            defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
            let updated = try await library.replaceIPA(of: app, with: file)
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
            guard device.method == .pairing, device.isReady else {
                return finish(id, app: updated, from: from, to: to, done: "Signed",
                              problem: "Signed \(to). Install it from the Library: installing over the web needs you to confirm.")
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

    private func finish(_ id: UUID, app: LibraryApp, from: String, to: String, done: String, problem: String?) -> Entry {
        status[id] = .done(done)
        return record(Entry(appName: app.displayName, from: from, to: to, date: Date(), problem: problem))
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
