import Foundation
import Observation
import UIKit

/// Decrypting an App Store app on this device (Pathway A,
/// docs/spikes/fairplay-ondevice.md), with no key: WarpSign puts the
/// encrypted app inside a small helper app (DecryptHelperTemplate.app,
/// carried in WarpSign), signs it, and the user installs it. Opened, the
/// helper has iOS decrypt the app with its own App Store licence (the Apple
/// ID that bought it has to be signed in to the App Store here), packs it
/// and hands it back: it serves the IPA on loopback and opens
/// `warpsign://decrypted?url=…&name=…`, which WarpSign downloads into the
/// library.
///
/// One decrypt at a time: the helper is the same app each time, installed
/// over the last.
@Observable
@MainActor
final class OnDeviceDecrypt {
    static let shared = OnDeviceDecrypt()

    enum Phase: Equatable {
        /// Putting the app in the helper, and signing it
        case preparing
        /// Signed: install it, then open it
        case ready(helper: URL)
        /// The helper was opened; waiting for it to hand the app back
        case waitingForHelper(helper: URL)
        /// Handed back: downloading it into the library
        case received(name: String)
        case failed(String)
    }

    /// The library app being decrypted, and how far it's got
    private(set) var appId: UUID?
    private(set) var phase: Phase?

    /// The scheme the helper answers to (ios/project.yml)
    static let helperScheme = "warpsign-decrypt"
    /// Where decrypted apps say they're from in the library
    static let origin = "Decrypted on This iPhone"

    private init() {}

    private static var folder: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("DecryptHelper", isDirectory: true)
    }

    /// Make and sign the helper for `app`, reporting signing's steps to
    /// `observer`
    func prepare(_ app: LibraryApp, account: AccountStore, observer: StepRecorder) async {
        appId = app.id
        phase = .preparing
        do {
            guard let template = Bundle.main.url(forResource: "DecryptHelperTemplate", withExtension: "app") else {
                throw WarpsignError.Ipa(message: "This build of WarpSign doesn't carry the decrypt helper.")
            }
            let folder = Self.folder
            try? FileManager.default.removeItem(at: folder)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            // The app and the helper around it, unpacked and packed
            try DiskSpace.ensure(Int64(clamping: app.installedSize &* 2 &+ app.fileSize), for: "Decrypting \(app.displayName)")

            let unsigned = folder.appendingPathComponent("Helper.ipa")
            let signed = folder.appendingPathComponent("Helper-signed.ipa")
            let encrypted = LibraryStore.ipaURL(of: app.id)
            let work = folder.appendingPathComponent("work", isDirectory: true)
            try await background {
                try buildDecryptHelper(
                    templateAppPath: template.path,
                    encryptedIpaPath: encrypted.path,
                    outputIpaPath: unsigned.path,
                    workDir: work.path
                )
            }
            try? FileManager.default.removeItem(at: work)

            // Signed as any app would be. Its URL scheme has to stay: it's
            // how WarpSign opens it.
            var options = SigningOptions()
            options.certificate = .development
            options.removeUrlSchemes = false
            _ = try await account.sign(
                ipa: unsigned,
                output: signed,
                kind: .development,
                settings: options.settings(icon: nil),
                observer: observer
            )
            try? FileManager.default.removeItem(at: unsigned)
            guard appId == app.id else { return }
            phase = .ready(helper: signed)
        } catch {
            guard appId == app.id else { return }
            phase = .failed(error.userMessage)
        }
    }

    /// Open the installed helper, which decrypts the app and hands it back
    func openHelper() async {
        guard case .ready(let helper) = phase,
              let link = URL(string: "\(Self.helperScheme)://decrypt") else { return }
        notInstalled = false
        if await UIApplication.shared.open(link) {
            phase = .waitingForHelper(helper: helper)
        } else {
            // Still signed: install it, and open it again
            notInstalled = true
        }
    }

    /// Opening the helper found it not installed
    private(set) var notInstalled = false

    /// Back to where the helper can be opened again (it decrypts each time
    /// it's opened)
    func retry() {
        guard case .waitingForHelper(let helper) = phase else { return }
        phase = .ready(helper: helper)
    }

    /// The helper's `warpsign://decrypted?url=…&name=…`. Only taken while a
    /// decrypt is waiting for it, and only from this device (loopback): a
    /// web page can open that link too.
    func receive(_ link: URL) -> Bool {
        guard case .waitingForHelper = phase,
              let items = URLComponents(url: link, resolvingAgainstBaseURL: false)?.queryItems,
              let address = items.first(where: { $0.name == "url" })?.value,
              let url = URL(string: address),
              url.scheme == "http",
              ["127.0.0.1", "localhost", "::1"].contains(url.host() ?? "")
        else { return false }
        let name = items.first(where: { $0.name == "name" })?.value ?? "App"
        DownloadManager.shared.download(url, name: name, purpose: .library(
            origin: Self.origin, sourceURL: nil, sourceAppId: nil, autoUpdate: false, link: nil
        ))
        phase = .received(name: name)
        return true
    }

    /// Done with it: forget the decrypt and its signed helper
    func finish() {
        try? FileManager.default.removeItem(at: Self.folder)
        appId = nil
        phase = nil
        notInstalled = false
    }
}
