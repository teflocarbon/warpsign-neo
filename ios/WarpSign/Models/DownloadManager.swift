import Foundation
import Observation

/// Every app WarpSign downloads: from sources, from a link, and updates.
/// Downloads run in a background URLSession, so they carry on while
/// WarpSign is in the background or suspended, and iOS relaunches WarpSign
/// to hand over one that finished after it was ended. Force-quitting
/// cancels them: they're kept, as interrupted, to resume where they
/// stopped when the server allows it. The list is saved, so nothing
/// vanishes without a word.
@MainActor
@Observable
final class DownloadManager {
    static let shared = DownloadManager()

    struct Item: Codable, Identifiable, Equatable {
        let id: UUID
        let url: URL
        /// The app's name, or the file's while it's not known
        var name: String
        let purpose: Purpose
        var state: State
        var received: Int64 = 0
        var total: Int64?
        /// Smoothed, while downloading
        var bytesPerSecond: Double?
        /// When bytes last arrived, to tell a stalled download
        var lastProgress: Date?
        var resumeData: Data?
        let started: Date

        var fraction: Double? {
            guard let total, total > 0 else { return nil }
            return min(Double(received) / Double(total), 1)
        }
    }

    enum Purpose: Codable, Equatable {
        /// Added to the library once downloaded
        case library(origin: String?, sourceURL: String?, sourceAppId: String?, autoUpdate: Bool?, link: String?)
        /// Handed to the Updater for this library app
        case update(appId: UUID)
    }

    enum State: Codable, Equatable {
        case downloading
        /// Downloaded, being added to the library
        case importing
        /// Stopped part-way: WarpSign was closed, or the user paused it
        case stopped(String)
        case failed(String)
    }

    private(set) var items: [Item] = []

    var activeCount: Int {
        items.filter { $0.state == .downloading || $0.state == .importing }.count
    }

    @ObservationIgnored private var library: LibraryStore?
    @ObservationIgnored private var session: URLSession?
    @ObservationIgnored private let delegate = SessionDelegate()
    /// Updates waiting on their download
    @ObservationIgnored private var waiting: [UUID: CheckedContinuation<URL, Error>] = [:]
    @ObservationIgnored private var progressHandlers: [UUID: (Double) -> Void] = [:]
    /// From iOS, when it relaunched WarpSign for finished downloads
    @ObservationIgnored var backgroundCompletion: (() -> Void)?

    nonisolated static let folder: URL = {
        var folder = URL.applicationSupportDirectory.appendingPathComponent("Downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        // Half-downloaded apps don't belong in the user's backups
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? folder.setResourceValues(values)
        return folder
    }()

    private static var listURL: URL { folder.appendingPathComponent("downloads.json") }

    private init() {}

    /// At launch (background launches too): reconnect to iOS's downloads
    func attach(library: LibraryStore) {
        guard session == nil, !Demo.isOn else { return }
        self.library = library
        if let data = try? Data(contentsOf: Self.listURL),
           let saved = try? JSONDecoder().decode([Item].self, from: data) {
            items = saved
        }
        delegate.manager = self
        let id = (Bundle.main.bundleIdentifier ?? "WarpSign") + ".downloads"
        let configuration = URLSessionConfiguration.background(withIdentifier: id)
        configuration.isDiscretionary = false
        configuration.sessionSendsLaunchEvents = true
        // Background sessions wait out a lost connection rather than fail;
        // give up after a day
        configuration.timeoutIntervalForResource = 24 * 60 * 60
        session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
        reconcile()
    }

    /// Downloads saved as running that iOS no longer has were cancelled
    /// with WarpSign (force-quit): mark them interrupted. Files left behind
    /// by nothing go.
    private func reconcile() {
        session?.getAllTasks { tasks in
            let live = Set(tasks.compactMap { $0.taskDescription.flatMap(UUID.init(uuidString:)) })
            Task { @MainActor in
                // Updates nothing's waiting for now: the next check finds them again
                self.items.removeAll { item in
                    guard case .update = item.purpose else { return false }
                    return self.waiting[item.id] == nil && !live.contains(item.id)
                }
                for index in self.items.indices where self.items[index].state == .downloading
                    && !live.contains(self.items[index].id) {
                    self.items[index].state = .stopped("Interrupted when WarpSign was closed.")
                }
                for index in self.items.indices where self.items[index].state == .importing {
                    self.items[index].state = .failed("Interrupted while adding it to the library. Download it again.")
                }
                self.save()
                self.removeStrayFiles()
            }
        }
    }

    // MARK: Starting

    /// Download an app into the library
    func download(_ url: URL, name: String, purpose: Purpose) {
        guard !items.contains(where: { $0.url == url && $0.state == .downloading }) else { return }
        // Getting it again replaces a stopped or failed try
        items.removeAll { $0.url == url }
        let item = Item(id: UUID(), url: url, name: name, purpose: purpose, state: .downloading, started: Date())
        items.insert(item, at: 0)
        start(item.id)
    }

    /// Download from a link the user gave: https only
    func download(link text: String) throws {
        var text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.contains("://") { text = "https://" + text }
        guard let url = URL(string: text), let host = url.host(), !host.isEmpty else {
            throw WarpsignError.Source(message: "That isn't a web address.")
        }
        guard url.scheme == "https" else {
            throw WarpsignError.Source(message: "WarpSign only downloads over https.")
        }
        let name = url.lastPathComponent.isEmpty || url.lastPathComponent == "/"
            ? host
            : (url.lastPathComponent.removingPercentEncoding ?? url.lastPathComponent)
        download(url, name: name, purpose: .library(origin: nil, sourceURL: nil, sourceAppId: nil, autoUpdate: nil, link: url.absoluteString))
    }

    /// Download an update and wait for the file, which the caller then owns
    func downloadUpdate(_ url: URL, name: String, appId: UUID, progress: @escaping (Double) -> Void) async throws -> URL {
        guard session != nil else { throw WarpsignError.Source(message: "Downloads aren't available.") }
        // An earlier one for this app goes: this replaces it
        items.removeAll { $0.purpose == .update(appId: appId) && $0.state != .downloading }
        let item = Item(id: UUID(), url: url, name: name, purpose: .update(appId: appId), state: .downloading, started: Date())
        items.insert(item, at: 0)
        progressHandlers[item.id] = progress
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                waiting[item.id] = continuation
                start(item.id)
            }
        } onCancel: {
            Task { @MainActor in self.cancel(item.id) }
        }
    }

    private func start(_ id: UUID) {
        guard let session, let index = items.firstIndex(where: { $0.id == id }) else { return }
        let task: URLSessionDownloadTask
        if let resumeData = items[index].resumeData {
            task = session.downloadTask(withResumeData: resumeData)
        } else {
            task = session.downloadTask(with: items[index].url)
            items[index].received = 0
            items[index].total = nil
        }
        task.taskDescription = id.uuidString
        items[index].resumeData = nil
        items[index].state = .downloading
        items[index].bytesPerSecond = nil
        items[index].lastProgress = Date()
        save()
        task.resume()
    }

    // MARK: Controls

    /// Stop, keeping what's downloaded to resume
    func pause(_ id: UUID) {
        task(for: id) { task in
            let paused: @Sendable (Data?) -> Void = { data in
                Task { @MainActor in
                    guard let index = self.items.firstIndex(where: { $0.id == id }) else { return }
                    self.items[index].resumeData = data
                    self.items[index].bytesPerSecond = nil
                    self.items[index].state = .stopped("Paused.")
                    self.save()
                }
            }
            if let task {
                task.cancel(byProducingResumeData: paused)
            } else {
                paused(nil)
            }
        }
    }

    /// Resume a stopped download, or try a failed one again
    func resume(_ id: UUID) {
        start(id)
    }

    /// Stop and forget it
    func cancel(_ id: UUID) {
        task(for: id) { $0?.cancel() }
        finish(id, with: .failure(CancellationError()))
        items.removeAll { $0.id == id }
        save()
    }

    func remove(_ id: UUID) {
        guard let item = items.first(where: { $0.id == id }), item.state != .downloading else { return }
        items.removeAll { $0.id == id }
        save()
    }

    private func task(for id: UUID, _ body: @escaping @Sendable (URLSessionDownloadTask?) -> Void) {
        guard let session else { return body(nil) }
        session.getAllTasks { tasks in
            body(tasks.first { $0.taskDescription == id.uuidString } as? URLSessionDownloadTask)
        }
    }

    // MARK: From the session

    fileprivate func progressed(_ id: UUID, received: Int64, total: Int64?) {
        guard let index = items.firstIndex(where: { $0.id == id }) else { return }
        let now = Date()
        let item = items[index]
        if let last = item.lastProgress, received > item.received {
            let seconds = now.timeIntervalSince(last)
            if seconds > 0 {
                let rate = Double(received - item.received) / seconds
                items[index].bytesPerSecond = item.bytesPerSecond.map { $0 * 0.7 + rate * 0.3 } ?? rate
            }
        }
        if received > item.received { items[index].lastProgress = now }
        items[index].received = received
        items[index].total = total
        if let fraction = items[index].fraction { progressHandlers[id]?(fraction) }
    }

    fileprivate func finished(_ id: UUID, file: URL, name: String?) {
        guard let index = items.firstIndex(where: { $0.id == id }) else {
            try? FileManager.default.removeItem(at: file)
            return
        }
        let item = items[index]
        switch item.purpose {
        case .update:
            if waiting[id] != nil {
                items.remove(at: index)
                save()
                finish(id, with: .success(file))
            } else {
                // WarpSign was relaunched for it, and nothing's waiting:
                // the next check finds the update again
                try? FileManager.default.removeItem(at: file)
                items.remove(at: index)
                save()
            }
        case let .library(origin, sourceURL, sourceAppId, autoUpdate, link):
            items[index].state = .importing
            if let name, case .library(_, _, _, _, .some) = item.purpose { items[index].name = name }
            save()
            Task {
                guard let library else { return }
                library.importError = nil
                let app = await library.importFile(
                    file,
                    origin: origin,
                    move: true,
                    sourceURL: sourceURL,
                    sourceAppId: sourceAppId,
                    autoUpdate: autoUpdate,
                    link: link
                )
                try? FileManager.default.removeItem(at: file)
                guard let current = items.firstIndex(where: { $0.id == id }) else { return }
                if app != nil {
                    items.remove(at: current)
                } else {
                    items[current].state = .failed(library.importError ?? "It isn't an app.")
                    library.importError = nil
                }
                save()
            }
        }
    }

    fileprivate func failed(_ id: UUID, message: String, resumeData: Data?, interrupted: Bool) {
        guard let index = items.firstIndex(where: { $0.id == id }) else { return }
        items[index].resumeData = resumeData
        items[index].bytesPerSecond = nil
        if interrupted, waiting[id] == nil, case .update = items[index].purpose {
            items[index].state = .failed("Interrupted. The next update check starts it again.")
        } else {
            items[index].state = interrupted ? .stopped(message) : .failed(message)
        }
        save()
        finish(id, with: .failure(WarpsignError.Source(message: message)))
    }

    fileprivate func sessionFinishedEvents() {
        backgroundCompletion?()
        backgroundCompletion = nil
    }

    private func finish(_ id: UUID, with result: Result<URL, Error>) {
        progressHandlers[id] = nil
        guard let continuation = waiting.removeValue(forKey: id) else { return }
        continuation.resume(with: result)
    }

    // MARK: Saving

    private func save() {
        guard !Demo.isOn, let data = try? JSONEncoder().encode(items) else { return }
        try? data.write(to: Self.listURL, options: .atomic)
    }

    /// Downloaded files no download owns (left by a crash, say)
    private func removeStrayFiles() {
        let owned = Set(items.map { $0.id.uuidString })
        let files = (try? FileManager.default.contentsOfDirectory(at: Self.folder, includingPropertiesForKeys: nil)) ?? []
        for file in files where file.lastPathComponent != Self.listURL.lastPathComponent {
            let id = file.deletingPathExtension().lastPathComponent
            if !owned.contains(id) {
                try? FileManager.default.removeItem(at: file)
            }
        }
    }
}

/// The background session's delegate, on its own queue. Moves each
/// finished file somewhere of WarpSign's at once (iOS deletes it when the
/// call returns), then hands over to the manager.
private final class SessionDelegate: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
    weak var manager: DownloadManager?
    private let lock = NSLock()
    /// When progress was last passed on, per download: at most 4 a second
    private var lastReport: [Int: Date] = [:]

    func urlSession(
        _ session: URLSession,
        downloadTask: URLSessionDownloadTask,
        didWriteData bytesWritten: Int64,
        totalBytesWritten: Int64,
        totalBytesExpectedToWrite: Int64
    ) {
        guard let id = downloadTask.taskDescription.flatMap(UUID.init(uuidString:)) else { return }
        let now = Date()
        let due = lock.withLock {
            if let last = lastReport[downloadTask.taskIdentifier], now.timeIntervalSince(last) < 0.25 { return false }
            lastReport[downloadTask.taskIdentifier] = now
            return true
        }
        guard due else { return }
        let total = totalBytesExpectedToWrite > 0 ? totalBytesExpectedToWrite : nil
        Task { @MainActor [weak manager] in
            manager?.progressed(id, received: totalBytesWritten, total: total)
        }
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        guard let id = downloadTask.taskDescription.flatMap(UUID.init(uuidString:)) else { return }
        let response = downloadTask.response as? HTTPURLResponse
        if let response, !(200..<300).contains(response.statusCode) {
            let message = "The server answered \(response.statusCode) (\(HTTPURLResponse.localizedString(forStatusCode: response.statusCode)))."
            report(id, message: message, resumeData: nil, interrupted: false)
            return
        }
        if let type = response?.mimeType, type.hasPrefix("text/html") {
            report(id, message: "That link opened a web page, not an app. Use a direct link to the .ipa.", resumeData: nil, interrupted: false)
            return
        }
        // Named as the server suggests, as .ipa unless it's a zip
        let suggested = response?.suggestedFilename ?? downloadTask.originalRequest?.url?.lastPathComponent ?? "Download.ipa"
        let ext = (suggested as NSString).pathExtension.lowercased() == "zip" ? "zip" : "ipa"
        let destination = DownloadManager.folder.appendingPathComponent("\(id.uuidString).\(ext)")
        do {
            try? FileManager.default.removeItem(at: destination)
            try FileManager.default.moveItem(at: location, to: destination)
        } catch {
            report(id, message: "Couldn't keep the download: \(error.localizedDescription)", resumeData: nil, interrupted: false)
            return
        }
        let name = (suggested as NSString).deletingPathExtension
        Task { @MainActor [weak manager] in
            manager?.finished(id, file: destination, name: name.isEmpty ? nil : name)
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.withLock { lastReport[task.taskIdentifier] = nil }
        guard let error, let id = task.taskDescription.flatMap(UUID.init(uuidString:)) else { return }
        let nsError = error as NSError
        let resumeData = nsError.userInfo[NSURLSessionDownloadTaskResumeData] as? Data
        if nsError.domain == NSURLErrorDomain, nsError.code == NSURLErrorCancelled {
            let reason = nsError.userInfo[NSURLErrorBackgroundTaskCancelledReasonKey] as? Int
            // Paused or cancelled here: handled where that was asked for
            guard let reason else { return }
            let message = reason == NSURLErrorCancelledReasonUserForceQuitApplication
                ? "Interrupted when WarpSign was closed."
                : "Interrupted by iOS."
            report(id, message: message, resumeData: resumeData, interrupted: true)
            return
        }
        report(id, message: Self.explain(nsError), resumeData: resumeData, interrupted: resumeData != nil)
    }

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        Task { @MainActor [weak manager] in manager?.sessionFinishedEvents() }
    }

    private func report(_ id: UUID, message: String, resumeData: Data?, interrupted: Bool) {
        Task { @MainActor [weak manager] in
            manager?.failed(id, message: message, resumeData: resumeData, interrupted: interrupted)
        }
    }

    private static func explain(_ error: NSError) -> String {
        guard error.domain == NSURLErrorDomain else { return error.localizedDescription }
        let host = (error.userInfo[NSURLErrorFailingURLErrorKey] as? URL)?.host() ?? "the server"
        switch error.code {
        case NSURLErrorNotConnectedToInternet: return "No internet connection."
        case NSURLErrorNetworkConnectionLost: return "The connection dropped."
        case NSURLErrorTimedOut: return "The server stopped answering."
        case NSURLErrorCannotFindHost, NSURLErrorDNSLookupFailed: return "Couldn't find \(host)."
        case NSURLErrorCannotConnectToHost: return "Couldn't connect to \(host)."
        case NSURLErrorAppTransportSecurityRequiresSecureConnection: return "WarpSign only downloads over https."
        case NSURLErrorServerCertificateUntrusted, NSURLErrorServerCertificateHasBadDate,
             NSURLErrorServerCertificateNotYetValid, NSURLErrorServerCertificateHasUnknownRoot,
             NSURLErrorSecureConnectionFailed:
            return "The server's certificate isn't trusted."
        case NSURLErrorCannotWriteToFile, NSURLErrorCannotCreateFile: return "There isn't room for it on this device."
        default: return error.localizedDescription
        }
    }
}
