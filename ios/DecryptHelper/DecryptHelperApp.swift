import SwiftUI
import UIKit

@main
struct DecryptHelperApp: App {
    var body: some Scene {
        WindowGroup { ContentView() }
    }
}

struct ContentView: View {
    @State private var report = "Decrypting…"
    var body: some View {
        VStack(spacing: 12) {
            Text("WarpSign Decrypt").font(.headline)
            ScrollView {
                Text(report)
                    .font(.system(.footnote, design: .monospaced))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .padding()
            }
        }
        .padding()
        .task { await runAndHandBack() }
    }

    func runAndHandBack() async {
        let result = await Task.detached { decryptEmbedded() }.value
        let docs = NSSearchPathForDirectoriesInDomains(.documentDirectory, .userDomainMask, true)[0]
        try? result.log.write(toFile: docs + "/report.txt", atomically: true, encoding: .utf8)
        report = result.log
        guard let app = result.decryptedApp else { return }
        report += "\n\n" + (await handBack(app: app, name: result.appName))
    }

    /// Package the decrypted app and hand it to WarpSign over loopback, the way
    /// CompanionHandoff hands an IPA to Companion (reversed).
    func handBack(app: String, name: String) async -> String {
        let docs = NSSearchPathForDirectoriesInDomains(.documentDirectory, .userDomainMask, true)[0]
        let ipa = docs + "/\(name)-decrypted.ipa"
        try? FileManager.default.removeItem(atPath: ipa)
        do {
            try packageIpa(appDir: app, outputIpa: ipa)
        } catch {
            return "package failed: \(error.localizedDescription)"
        }
        do {
            let slot = UInt32.random(in: 0..<webInstallSlots())
            let share = try LoopbackShare.start(ipaPath: ipa, slot: slot, observer: StepRecorder { _ in })
            KeepAlive.shared.start("decrypt")
            var components = URLComponents()
            components.scheme = "warpsign"
            components.host = "decrypted"
            components.queryItems = [
                URLQueryItem(name: "url", value: share.url()),
                URLQueryItem(name: "name", value: name),
            ]
            guard let link = components.url, await UIApplication.shared.open(link) else {
                share.stop()
                KeepAlive.shared.stop("decrypt")
                return "couldn't reach WarpSign"
            }
            // Keep serving a while so WarpSign can fetch it in the foreground.
            Task {
                try? await Task.sleep(for: .seconds(300))
                share.stop()
                KeepAlive.shared.stop("decrypt")
            }
            return "handed \(name)-decrypted.ipa back to WarpSign"
        } catch {
            return "serve failed: \(error.localizedDescription)"
        }
    }
}

struct DecryptResult {
    var log: String
    var decryptedApp: String?
    var appName: String
}

/// Decrypt every FairPlay-encrypted Mach-O in the app embedded at
/// `EmbeddedPayload/<App>/` into a copy under Documents/decrypted/.
func decryptEmbedded() -> DecryptResult {
    let fm = FileManager.default
    let embeddedRoot = Bundle.main.bundlePath + "/EmbeddedPayload"
    guard let entries = try? fm.contentsOfDirectory(atPath: embeddedRoot),
          let appDir = entries.first(where: { !$0.hasPrefix(".") })
    else { return DecryptResult(log: "No EmbeddedPayload/<App> in bundle", decryptedApp: nil, appName: "") }

    let srcApp = embeddedRoot + "/" + appDir
    let docs = NSSearchPathForDirectoriesInDomains(.documentDirectory, .userDomainMask, true)[0]
    let outRoot = docs + "/decrypted"
    let dstApp = outRoot + "/Payload/" + appDir + ".app"
    try? fm.removeItem(atPath: outRoot)
    try? fm.createDirectory(atPath: dstApp, withIntermediateDirectories: true)

    var log = "embedded: \(appDir)\n"
    var decrypted = 0, failed = 0, copyErr = 0
    guard let walker = fm.enumerator(atPath: srcApp) else {
        return DecryptResult(log: "cannot enumerate \(srcApp)", decryptedApp: nil, appName: appDir)
    }
    while let rel = walker.nextObject() as? String {
        if (rel as NSString).lastPathComponent == "SC_Info" { walker.skipDescendants(); continue }
        let src = srcApp + "/" + rel
        let dst = dstApp + "/" + rel
        var isDir: ObjCBool = false
        fm.fileExists(atPath: src, isDirectory: &isDir)
        if isDir.boolValue { try? fm.createDirectory(atPath: dst, withIntermediateDirectories: true); continue }
        do { try fm.copyItem(atPath: src, toPath: dst) }
        catch { copyErr += 1; log += "  x \(rel): copy \(error.localizedDescription)\n"; continue }
        var msg = [CChar](repeating: 0, count: 256)
        let rc = dh_decrypt_into(src, dst, &msg, 256)
        if rc == 1 { decrypted += 1 } else if rc == -1 { failed += 1; log += "  ! \(rel): \(String(cString: msg))\n" }
    }
    log += "DONE  decrypted=\(decrypted)  failed=\(failed)  copyErrors=\(copyErr)"
    let ok = failed == 0 && copyErr == 0
    return DecryptResult(log: log, decryptedApp: ok ? dstApp : nil, appName: appDir)
}
