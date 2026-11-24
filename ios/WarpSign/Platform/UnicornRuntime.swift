import CryptoKit
import Foundation
import UIKit

/// Unicorn, the emulator ipatool runs Apple's code in to sign in and
/// download. It's GPLv2-only, so WarpSign doesn't ship it
/// (ipatool-bridge/unicorn/README.md): it's downloaded once, checked
/// against the hash here, signed with the user's development certificate
/// (iOS only loads code signed by the app's own team) and loaded.
///
/// The build is Unicorn with QEMU's interpreter (interpreter.patch): it runs
/// Apple's code without JIT, so nothing else is needed. A JIT build (split
/// W^X, splitwx.patch) would also load, and then asks StikDebug for JIT
/// memory. See crates/warpsign-ffi/src/emulator.rs.
enum UnicornRuntime {
    /// Unicorn's version, then the interpreter patch's
    static let version = "2.1.4-tci1"

    /// The build for this platform: built by ipatool-bridge/unicorn/build.sh
    /// and published with the release named after the version
    private static let file: (name: String, sha256: String) = {
        #if targetEnvironment(simulator)
        ("libunicorn-iphonesimulator.dylib", "77c4c595f949249a04e88dec8c1e89aeddb7682811073ce11f7f193dd2af45ed")
        #else
        ("libunicorn-iphoneos.dylib", "e9925081eebdfbffe4cbf1e9dd8797ee26ad393da6cff7dd79aca7d6a08f03bf")
        #endif
    }()

    private static var url: URL {
        URL(string: "https://github.com/teflocarbon/warpsign-neo/releases/download/unicorn-\(version)/\(file.name)")!
    }

    static var directory: URL {
        AppStoreEngine.home.appendingPathComponent("Unicorn/\(version)", isDirectory: true)
    }

    /// As downloaded and checked
    private static var original: URL { directory.appendingPathComponent("libunicorn-unsigned.dylib") }
    /// Signed for this team, and loaded
    private static var signed: URL { directory.appendingPathComponent("libunicorn.dylib") }

    enum Failure: LocalizedError {
        case download(String)
        case changed
        case noCertificate
        case otherTeam(app: String, certificate: String)
        case noStikDebug
        case noDebugger
        case wrongResult(UInt64)
        case notLoaded(String)

        var errorDescription: String? {
            switch self {
            case .download(let reason):
                "Couldn't download Unicorn: \(reason)"
            case .changed:
                "The Unicorn download isn't the expected file, so it wasn't used."
            case .noCertificate:
                "Unicorn has to be signed with your development certificate. Sign in and make one first."
            case .otherTeam(let app, let certificate):
                "WarpSign is signed by team \(app), but your development certificate is for team \(certificate). iOS only loads Unicorn signed by WarpSign's own team."
            case .noStikDebug:
                "JIT needs StikDebug. Install it, or enable JIT for WarpSign another way, and try again."
            case .noDebugger:
                "JIT wasn't enabled: StikDebug didn't attach to WarpSign. Make sure LocalDevVPN is connected, then try again."
            case .wrongResult(let result):
                "Unicorn ran its check wrongly (got \(result), not 3000)."
            case .notLoaded(let tried):
                "iOS wouldn't load Unicorn:\n\(tried)"
            }
        }
    }

    static var isReady: Bool { emulatorState().ready }

    /// Downloaded before: setting up only signs and loads it
    static var isDownloaded: Bool { FileManager.default.fileExists(atPath: original.path) }

    /// Get Unicorn ready for ipatool: download, sign and load it, and check
    /// it runs. Each step is skipped once done. Only a JIT build needs more:
    /// it opens StikDebug, and waits for it to attach (up to `wait`).
    @MainActor
    static func prepare(
        identity: IdentityData,
        wait: Duration = .seconds(90),
        report: @escaping (String) -> Void
    ) async throws {
        if !emulatorState().loaded {
            try await loadLibrary(identity: identity, report: report)
        }
        // The interpreter is ready once loaded
        if !emulatorState().ready {
            try await setUpJit(wait: wait, report: report)
        }
        let result = try await background { try checkUnicorn() }
        guard result == 3000 else { throw Failure.wrongResult(result) }
        report("Unicorn is ready")
    }

    /// A JIT build's memory, from StikDebug on iOS 26 and later
    @MainActor
    private static func setUpJit(wait: Duration, report: @escaping (String) -> Void) async throws {
        #if targetEnvironment(simulator)
        // The simulator runs on the Mac: no debugger needed
        report("Setting up JIT memory")
        try await background { try WarpSign.setUpJit(throughDebugger: false) }
        #else
        // iOS 26's TXM: executable memory only through the debugger's
        // script, which has to be attached right now. JIT enabled ahead in
        // StikDebug leaves WarpSign debugged with nothing attached, so ask
        // again. Before 26, having been debugged is enough.
        let throughDebugger = ProcessInfo.processInfo.isOperatingSystemAtLeast(
            OperatingSystemVersion(majorVersion: 26, minorVersion: 0, patchVersion: 0))
        func jitAllowed() -> Bool {
            let state = emulatorState()
            return throughDebugger ? state.attached && state.debugged : state.debugged
        }
        if !jitAllowed() {
            report("Asking StikDebug for JIT")
            try await askStikDebug()
            let deadline = ContinuousClock.now + wait
            while !jitAllowed() {
                guard ContinuousClock.now < deadline else { throw Failure.noDebugger }
                try await Task.sleep(for: .milliseconds(500))
            }
        }
        report("Setting up JIT memory")
        try await background { try WarpSign.setUpJit(throughDebugger: throughDebugger) }
        #endif
    }

    private static func loadLibrary(identity: IdentityData, report: @escaping (String) -> Void) async throws {
        let certificateTeam = try identityInfo(identity: identity).teamId
        guard let certificateTeam else { throw Failure.noCertificate }
        if let appTeam = appTeamId, appTeam != certificateTeam {
            throw Failure.otherTeam(app: appTeam, certificate: certificateTeam)
        }

        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        if !(try await background { isExpected(original) }) {
            report("Downloading Unicorn")
            try await download()
        }
        report("Signing Unicorn")
        try await background {
            let files = FileManager.default
            try? files.removeItem(at: signed)
            // A fresh file, not a copy: nothing of the download's (extended
            // attributes such as a quarantine mark) carries over
            try Data(contentsOf: original).write(to: signed)
            let attributes = extendedAttributes(signed)
            attributes.forEach { removeExtendedAttribute($0, from: signed) }
            try signLibrary(
                path: signed.path,
                identity: identity,
                teamId: certificateTeam,
                identifier: "org.unicorn-engine.unicorn"
            )
            try loadFirstAllowed(attributes: attributes)
        }
    }

    /// Load the signed library from the first place iOS allows it.
    ///
    /// With no debugger attached, iOS's sandbox refused to map the library
    /// when it was a copy of the download ("file system sandbox blocked
    /// mmap()"). A freshly written file, with no extended attributes, loaded
    /// (seen on a device in November 2026). Other folders are kept as
    /// fallbacks, and if none loads, the error says what happened in each.
    private static func loadFirstAllowed(attributes: [String]) throws {
        let files = FileManager.default
        let places: [(String, URL)] = [
            ("Application Support", signed),
            ("tmp", files.temporaryDirectory.appendingPathComponent("unicorn", isDirectory: true)),
            ("Caches", URL.cachesDirectory.appendingPathComponent("unicorn", isDirectory: true)),
            ("Documents", URL.documentsDirectory.appendingPathComponent(".unicorn", isDirectory: true)),
        ]
        var tried: [String] = []
        for (name, place) in places {
            var file = place
            do {
                if place != signed {
                    try files.createDirectory(at: place, withIntermediateDirectories: true)
                    file = place.appendingPathComponent("libunicorn.dylib")
                    try? files.removeItem(at: file)
                    try Data(contentsOf: signed).write(to: file)
                }
                try loadUnicorn(path: file.path)
                return
            } catch {
                tried.append("\(name): \(shortLoadError(error.userMessage))")
            }
        }
        let debugged = emulatorState().debugged ? "yes" : "no"
        throw Failure.notLoaded(
            tried.joined(separator: "\n")
                + "\nAttributes removed: \(attributes.isEmpty ? "none" : attributes.joined(separator: ", "))"
                + "\nDebugged: \(debugged), iOS \(UIDevice.current.systemVersion)")
    }

    /// dyld's reason, without the paths it tried
    private static func shortLoadError(_ message: String) -> String {
        if message.contains("sandbox blocked mmap") { return "sandbox blocked it" }
        // e.g. "… (code signature in <…> '…' not valid for use in process: …)"
        if let reason = message.range(of: "code signature") ?? message.range(of: "not valid") {
            return String(message[reason.lowerBound...].prefix(160))
        }
        return String(message.prefix(160))
    }

    private static func extendedAttributes(_ file: URL) -> [String] {
        file.withUnsafeFileSystemRepresentation { path -> [String] in
            guard let path else { return [] }
            let size = listxattr(path, nil, 0, 0)
            guard size > 0 else { return [] }
            var names = [CChar](repeating: 0, count: size)
            guard listxattr(path, &names, size, 0) == size else { return [] }
            return names.split(separator: 0).map { name in
                String(decoding: name.map { UInt8(bitPattern: $0) }, as: UTF8.self)
            }
        }
    }

    private static func removeExtendedAttribute(_ name: String, from file: URL) {
        _ = file.withUnsafeFileSystemRepresentation { path in
            path.map { removexattr($0, name, 0) }
        }
    }

    private static func download() async throws {
        let (temporary, response): (URL, URLResponse)
        do {
            (temporary, response) = try await URLSession.shared.download(from: url)
        } catch {
            throw Failure.download(error.localizedDescription)
        }
        if let http = response as? HTTPURLResponse, http.statusCode != 200 {
            throw Failure.download("HTTP \(http.statusCode)")
        }
        guard isExpected(temporary) else {
            try? FileManager.default.removeItem(at: temporary)
            throw Failure.changed
        }
        try? FileManager.default.removeItem(at: original)
        try FileManager.default.moveItem(at: temporary, to: original)
    }

    /// Whether the file is the build pinned above
    private static func isExpected(_ file: URL) -> Bool {
        guard let data = try? Data(contentsOf: file, options: .mappedIfSafe) else { return false }
        let hash = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        return hash == self.file.sha256
    }

    /// The team in WarpSign's own provisioning profile (none in the simulator)
    private static var appTeamId: String? {
        guard let path = Bundle.main.path(forResource: "embedded", ofType: "mobileprovision") else { return nil }
        return (try? readProfile(path: path))?.teamIds.first
    }

    /// StikDebug's JIT request (StikJIT's INTEGRATION.md): this app and
    /// process, and on iOS 26 and later the script that knows the breakpoint
    /// protocol emulator.rs uses
    @MainActor
    private static func askStikDebug() async throws {
        var components = URLComponents()
        components.scheme = "stikdebug"
        components.host = "enable-jit"
        components.queryItems = [
            URLQueryItem(name: "bundle-id", value: Bundle.main.bundleIdentifier),
            URLQueryItem(name: "pid", value: String(ProcessInfo.processInfo.processIdentifier)),
        ]
        if ProcessInfo.processInfo.isOperatingSystemAtLeast(
            OperatingSystemVersion(majorVersion: 26, minorVersion: 0, patchVersion: 0)) {
            components.queryItems?.append(URLQueryItem(name: "script-name", value: "universal.js"))
        }
        guard let link = components.url, await UIApplication.shared.open(link) else {
            throw Failure.noStikDebug
        }
    }
}
