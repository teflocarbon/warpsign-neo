import Foundation

/// ipatool's App Store client, linked into the app (ipatool-bridge; see
/// crates/warpsign-ffi/src/appstore.rs). Its calls block: use it off the
/// main thread.
enum AppStoreEngine {
    private static let passphraseKey = "appstore.passphrase"
    private static let macAddressKey = "appstore.macAddress"
    private static let lock = NSLock()
    nonisolated(unsafe) private static var shared: AppStoreClient?

    /// Whether this build has ipatool in it
    static var isAvailable: Bool { appStoreAvailable() }

    /// Where ipatool keeps its session (encrypted) and cookies
    static var home: URL {
        URL.applicationSupportDirectory.appendingPathComponent("AppStore", isDirectory: true)
    }

    static func client() throws -> AppStoreClient {
        lock.lock()
        defer { lock.unlock() }
        if let shared { return shared }
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        keepLastCrash()
        let client = try AppStoreClient(
            home: home.path,
            passphrase: try secret(passphraseKey) { randomBytes(32).base64EncodedString() },
            macAddress: try secret(macAddressKey) { madeUpMacAddress() }
        )
        shared = client
        return client
    }

    /// Go's report of the last time ipatool or Unicorn crashed the app,
    /// which iOS keeps no report of (bridge.go)
    static var lastCrash: String? {
        guard let text = try? String(contentsOf: lastCrashFile, encoding: .utf8) else { return nil }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    static func forgetLastCrash() {
        try? FileManager.default.removeItem(at: lastCrashFile)
    }

    private static var lastCrashFile: URL { home.appendingPathComponent("last-crash.log") }

    /// Before the bridge starts writing a new one
    private static func keepLastCrash() {
        let crash = home.appendingPathComponent("ipatool/crash.log")
        guard let size = (try? crash.resourceValues(forKeys: [.fileSizeKey]))?.fileSize, size > 0 else { return }
        try? FileManager.default.removeItem(at: lastCrashFile)
        try? FileManager.default.moveItem(at: crash, to: lastCrashFile)
    }

    /// A value kept in the Keychain, made the first time
    private static func secret(_ key: String, make: () -> String) throws -> String {
        if let data = Keychain.data(for: key), let value = String(data: data, encoding: .utf8) {
            return value
        }
        let value = make()
        try Keychain.set(Data(value.utf8), for: key)
        return value
    }

    /// iOS gives apps no MAC address, and Apple knows a device by one, so
    /// make one up (locally administered, unicast) and keep it
    private static func madeUpMacAddress() -> String {
        var bytes = randomBytes(6)
        bytes[0] = (bytes[0] & 0xFC) | 0x02
        return bytes.map { String(format: "%02x", $0) }.joined(separator: ":")
    }

    #if DEBUG
    /// With the launch argument --appstore-self-test: prepare ipatool and
    /// print how it went, for trying it from a terminal
    /// (`xcrun simctl launch --console … --appstore-self-test`). No Apple ID.
    static func selfTestIfAsked() {
        guard ProcessInfo.processInfo.arguments.contains("--appstore-self-test") else { return }
        Thread.detachNewThread {
            let started = Date()
            func report(_ line: String) {
                print(String(format: "[appstore-self-test %.1fs] ", Date().timeIntervalSince(started)) + line)
                fflush(stdout)
            }
            report("available: \(isAvailable)")
            // --unicorn <path>: a Unicorn already signed for this machine
            let arguments = ProcessInfo.processInfo.arguments
            if let index = arguments.firstIndex(of: "--unicorn"), index + 1 < arguments.count {
                do {
                    try loadUnicorn(path: arguments[index + 1])
                    report("unicorn: loaded")
                    if !emulatorState().interpreter {
                        try setUpJit(throughDebugger: false)
                        report("unicorn: JIT memory set up")
                    }
                    report("unicorn: check gave \(try checkUnicorn()) (expected 3000)")
                } catch {
                    report("unicorn failed: \(error.userMessage)")
                }
            }
            do {
                let client = try client()
                report("configured, home \(home.path)")
                try client.prepare()
                report("prepare: ok")
                report("account: \(try client.account() == nil ? "not signed in" : "signed in")")
            } catch {
                report("failed: \(error.userMessage)")
            }
            report("done")
        }
    }
    #endif

    private static func randomBytes(_ count: Int) -> Data {
        var bytes = Data(count: count)
        let status = bytes.withUnsafeMutableBytes {
            SecRandomCopyBytes(kSecRandomDefault, count, $0.baseAddress!)
        }
        precondition(status == errSecSuccess, "no random bytes")
        return bytes
    }
}
