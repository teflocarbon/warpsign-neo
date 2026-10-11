import SwiftUI
import UIKit

/// Sample data and a starting screen for screenshots: launch with
/// `-demo -demoScreen <name>` (see tools/screenshots.sh). Never touches
/// the Keychain, the network or the real library.
enum Demo {
    static let isOn = ProcessInfo.processInfo.arguments.contains("-demo")

    enum Screen: String {
        case library, appDetail, signOptions, signing, install
        case sources, sourceDetail
        case settings, installing, signIn
    }

    static var screen: Screen {
        let arguments = ProcessInfo.processInfo.arguments
        guard let index = arguments.firstIndex(of: "-demoScreen"), index + 1 < arguments.count else {
            return .library
        }
        return Screen(rawValue: arguments[index + 1]) ?? .library
    }

    // MARK: Library

    struct SampleApp {
        let name: String
        let bundleId: String
        let version: String
        let size: UInt64
        let symbol: String
        let colors: [UIColor]
        var encrypted = false
        var signed = false
    }

    static let samples: [SampleApp] = [
        SampleApp(name: "Lumen", bundleId: "com.lumen.camera", version: "3.2.1", size: 84_200_000,
                  symbol: "camera.aperture", colors: [.systemOrange, .systemPink], signed: true),
        SampleApp(name: "Tidepool", bundleId: "io.tidepool.weather", version: "1.8", size: 31_500_000,
                  symbol: "cloud.sun.fill", colors: [.systemCyan, .systemBlue]),
        SampleApp(name: "Arcade 64", bundleId: "dev.arcade64.emulator", version: "2.0.3", size: 152_000_000,
                  symbol: "gamecontroller.fill", colors: [.systemRed, .systemPurple], signed: true),
        SampleApp(name: "Notary", bundleId: "app.notary.notes", version: "5.4", size: 18_900_000,
                  symbol: "pencil.and.scribble", colors: [.systemGreen, .systemTeal]),
        SampleApp(name: "Streamline", bundleId: "tv.streamline.player", version: "12.1.0", size: 210_000_000,
                  symbol: "play.rectangle.fill", colors: [.systemIndigo, .systemBlue], encrypted: true),
    ]

    static func icon(_ symbol: String, _ colors: [UIColor], size: CGFloat = 180) -> UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: size, height: size)).image { context in
            let gradient = CGGradient(
                colorsSpace: CGColorSpaceCreateDeviceRGB(),
                colors: colors.map(\.cgColor) as CFArray,
                locations: [0, 1]
            )!
            context.cgContext.drawLinearGradient(
                gradient,
                start: .zero,
                end: CGPoint(x: size, y: size),
                options: []
            )
            let configuration = UIImage.SymbolConfiguration(pointSize: size * 0.42, weight: .semibold)
            if let glyph = UIImage(systemName: symbol, withConfiguration: configuration)?
                .withTintColor(.white, renderingMode: .alwaysOriginal) {
                let origin = CGPoint(x: (size - glyph.size.width) / 2, y: (size - glyph.size.height) / 2)
                glyph.draw(at: origin)
            }
        }
    }

    /// Stable IDs, so screens can link to an app
    static let appIds: [UUID] = samples.indices.map { index in
        UUID(uuidString: String(format: "00000000-0000-4000-8000-%012d", index + 1))!
    }

    static func library() -> LibraryStore {
        var apps: [LibraryApp] = []
        var icons: [UUID: UIImage] = [:]
        for (index, (sample, id)) in zip(samples, appIds).enumerated() {
            var app = LibraryApp(
                id: id,
                displayName: sample.name,
                bundleId: sample.bundleId,
                version: sample.version,
                build: sample.version.replacingOccurrences(of: ".", with: ""),
                minimumOS: "16.0",
                fileSize: sample.size,
                installedSize: sample.size * 2,
                extensionCount: sample.signed ? 2 : 0,
                encryption: sample.encrypted ? "encrypted" : "decrypted",
                addedAt: Date(timeIntervalSinceNow: -86_400 * Double(index + 1)),
                origin: sample.name == "Arcade 64" ? "Example Repo" : nil
            )
            if sample.signed {
                app.signedCopies = [signedCopy(for: sample)]
            }
            apps.append(app)
            icons[id] = icon(sample.symbol, sample.colors)
        }
        return LibraryStore(demoApps: apps, icons: icons)
    }

    static func signedCopy(for sample: SampleApp) -> SignedCopy {
        SignedCopy(
            id: UUID(),
            date: Date(timeIntervalSinceNow: -3_600),
            fileName: "\(sample.name) \(sample.version) signed.ipa",
            bundleId: "4nq.qv4clrx.\(sample.name.lowercased().filter(\.isLetter))",
            teamId: team.teamId,
            certificate: "Development",
            ppqCheck: false,
            profileExpires: Date(timeIntervalSinceNow: 86_400 * 364)
        )
    }

    // MARK: Account

    static let team = TeamInfo(teamId: "ZZZZZ99999", name: "Teflo Labs", kind: "INDIVIDUAL", roles: ["TEAM_ADMIN"])

    static func account() -> AccountStore {
        let expires = Int64(Date(timeIntervalSinceNow: 86_400 * 300).timeIntervalSince1970)
        return AccountStore(
            demoAppleId: "teflo@example.com",
            team: team,
            identities: [
                .development: IdentityInfo(
                    commonName: "Apple Development: Teflo (ABCDE12345)",
                    kind: .development, teamId: team.teamId,
                    serialNumber: "43BEABBF257C8E9218D0C2F136076B9D", expiresAt: expires
                ),
                .distribution: IdentityInfo(
                    commonName: "Apple Distribution: Teflo Labs (ZZZZZ99999)",
                    kind: .distribution, teamId: team.teamId,
                    serialNumber: "5A1B2C3D4E5F60718293A4B5C6D7E8F9", expiresAt: expires
                ),
            ]
        )
    }

    // MARK: Sources

    static let sourceURL = "https://repo.example.com/apps.json"

    static func sources() -> SourcesStore {
        func iconURL(_ symbol: String, _ colors: [UIColor]) -> String {
            let data = icon(symbol, colors, size: 120).pngData() ?? Data()
            return "data:image/png;base64,\(data.base64EncodedString())"
        }
        struct SourceSample {
            let name: String
            let id: String
            let developer: String
            let subtitle: String
            let symbol: String
            let colors: [UIColor]
            let version: String
            let size: Int
        }
        let samples: [SourceSample] = [
            SourceSample(name: "Arcade 64", id: "dev.arcade64.emulator", developer: "Arcade Dev",
                         subtitle: "Retro consoles, on the go", symbol: "gamecontroller.fill",
                         colors: [.systemRed, .systemPurple], version: "2.0.3", size: 152_000_000),
            SourceSample(name: "Lumen", id: "com.lumen.camera", developer: "Lumen Studio",
                         subtitle: "Manual camera controls", symbol: "camera.aperture",
                         colors: [.systemOrange, .systemPink], version: "3.2.1", size: 84_200_000),
            SourceSample(name: "Fieldnotes", id: "app.fieldnotes", developer: "Paper Co",
                         subtitle: "Notes that sync anywhere", symbol: "book.closed.fill",
                         colors: [.systemBrown, .systemOrange], version: "1.4", size: 22_000_000),
            SourceSample(name: "Signal Boost", id: "io.signalboost.radio", developer: "Waveform",
                         subtitle: "Internet radio, offline", symbol: "antenna.radiowaves.left.and.right",
                         colors: [.systemPink, .systemPurple], version: "4.0", size: 40_100_000),
            SourceSample(name: "Orbit", id: "space.orbit.tracker", developer: "Night Sky",
                         subtitle: "Satellites overhead, live", symbol: "globe.americas.fill",
                         colors: [.systemBlue, .systemIndigo], version: "0.9.2", size: 15_700_000),
        ]
        var apps: [[String: Any]] = []
        for sample in samples {
            let version: [String: Any] = [
                "version": sample.version,
                "date": "2026-09-20",
                "size": sample.size,
                "downloadURL": "https://repo.example.com/\(sample.id).ipa",
                "localizedDescription": "Bug fixes and improvements.",
            ]
            apps.append([
                "name": sample.name,
                "bundleIdentifier": sample.id,
                "developerName": sample.developer,
                "subtitle": sample.subtitle,
                "iconURL": iconURL(sample.symbol, sample.colors),
                "localizedDescription": "\(sample.name) is a sample app, shown for screenshots.",
                "versions": [version],
            ])
        }
        let json: [String: Any] = [
            "name": "Example Repo",
            "identifier": "com.example.repo",
            "subtitle": "Handpicked apps for testing",
            "iconURL": iconURL("square.stack.3d.up.fill", [.systemPurple, .systemBlue]),
            "tintColor": "#7B3FE4",
            "featuredApps": ["dev.arcade64.emulator"],
            "apps": apps,
        ]
        let data = (try? JSONSerialization.data(withJSONObject: json)) ?? Data()
        let source = try? parseSource(data: data)
        return SourcesStore(demoSources: source.map { [sourceURL: $0] } ?? [:])
    }

    // MARK: Signing

    static var signingResult: (SignResult, SignedCopy, [StepState]) {
        let sample = samples[0]
        let copy = signedCopy(for: sample)
        let logs: [Step: [String]] = [
            .extract: ["Lumen.ipa (84.2 MB)", "Found Lumen.app"],
            .inspectComponents: ["App: com.lumen.camera (6 entitlements)", "Extension PlugIns/Widget.appex: com.lumen.camera.widget (3 entitlements)"],
            .readTeamIds: ["QWERT67890"],
            .planIdentifiers: ["App: com.lumen.camera → \(copy.bundleId)", "  capabilities: APP_GROUPS, PUSH_NOTIFICATIONS"],
            .registerIdentifiers: ["App group group.\(copy.bundleId): 7XK2P9", "App ID \(copy.bundleId): BNDL01"],
            .createProfiles: ["1 device", "App: TS \(copy.bundleId) Development (regenerated)"],
            .sign: ["Signed with Apple Development: Teflo (ABCDE12345)"],
            .package: ["Lumen 3.2.1 signed.ipa (84.3 MB)"],
        ]
        let times: [UInt64] = [820, 140, 12, 960, 2_310, 25, 1_870, 140, 3_920, 2_640]
        let steps = signSteps().enumerated().map { index, info in
            StepState(step: info.step, title: info.title, status: .done(milliseconds: times[index % times.count]), log: logs[info.step] ?? [])
        }
        let result = SignResult(
            outputPath: "/tmp/Lumen.ipa",
            bundleId: copy.bundleId,
            profiles: [
                SignedProfileInfo(bundlePath: "", bundleId: copy.bundleId, name: "TS \(copy.bundleId) Development", ppqCheck: false, expiresAt: Int64(Date(timeIntervalSinceNow: 86_400 * 364).timeIntervalSince1970)),
                SignedProfileInfo(bundlePath: "PlugIns/Widget.appex", bundleId: "\(copy.bundleId).widget", name: "TS \(copy.bundleId).widget Development", ppqCheck: false, expiresAt: Int64(Date(timeIntervalSinceNow: 86_400 * 364).timeIntervalSince1970)),
            ],
            warnings: []
        )
        return (result, copy, steps)
    }

    static var installSteps: [StepState] {
        webInstallSteps().enumerated().map { index, info in
            StepState(step: info.step, title: info.title, status: index == 0 ? .running : .pending,
                      log: index == 0 ? ["Tap Install when iOS asks. Keep WarpSign open until the app is sent."] : [])
        }
    }
}
