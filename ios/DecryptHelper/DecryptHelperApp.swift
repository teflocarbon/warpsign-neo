import SwiftUI

@main
struct DecryptHelperApp: App {
    var body: some Scene {
        WindowGroup { ContentView() }
    }
}

struct ContentView: View {
    @State private var report = "Decrypting…"
    var body: some View {
        ScrollView {
            Text(report)
                .font(.system(.footnote, design: .monospaced))
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
                .padding()
        }
        .task {
            let r = await Task.detached { decryptEmbedded() }.value
            let docs = NSSearchPathForDirectoriesInDomains(.documentDirectory, .userDomainMask, true)[0]
            try? r.write(toFile: docs + "/report.txt", atomically: true, encoding: .utf8)
            report = r
        }
    }
}

/// Decrypt every FairPlay-encrypted Mach-O in the app embedded at
/// `EmbeddedPayload/<App>/` into a copy under Documents/decrypted/.
func decryptEmbedded() -> String {
    let fm = FileManager.default
    let embeddedRoot = Bundle.main.bundlePath + "/EmbeddedPayload"
    guard let entries = try? fm.contentsOfDirectory(atPath: embeddedRoot),
          let appDir = entries.first(where: { !$0.hasPrefix(".") })
    else { return "No EmbeddedPayload/<App> in bundle" }

    let srcApp = embeddedRoot + "/" + appDir
    let docs = NSSearchPathForDirectoriesInDomains(.documentDirectory, .userDomainMask, true)[0]
    let outRoot = docs + "/decrypted"
    let dstApp = outRoot + "/Payload/" + appDir + ".app"
    try? fm.removeItem(atPath: outRoot)
    try? fm.createDirectory(atPath: dstApp, withIntermediateDirectories: true)

    // Rebuild the app under Documents, skipping SC_Info (DRM metadata the app
    // can't read — only fairplayd can, and the decrypted output doesn't need
    // it), and decrypt each encrypted binary from the read-only bundle copy.
    var log = "embedded: \(appDir)\n"
    var decrypted = 0, failed = 0, copyErr = 0
    guard let walker = fm.enumerator(atPath: srcApp) else { return "cannot enumerate \(srcApp)" }
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
        if rc == 1 { decrypted += 1; log += "  + \(rel): \(String(cString: msg))\n" }
        else if rc == -1 { failed += 1; log += "  ! \(rel): \(String(cString: msg))\n" }
    }
    log += "\nDONE  decrypted=\(decrypted)  failed=\(failed)  copyErrors=\(copyErr)\noutput: \(dstApp)"
    return log
}
