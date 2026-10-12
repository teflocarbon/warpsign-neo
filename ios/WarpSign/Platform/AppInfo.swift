import Foundation

/// This app's version, build and commit, to tell builds apart. CI sets the
/// build to the IPA workflow's run number and the commit to the one built
/// (`.github/workflows/ipa.yml`); local builds are build 1, commit "local".
enum AppInfo {
    static var version: String { string("CFBundleShortVersionString") ?? "?" }
    static var build: String { string("CFBundleVersion") ?? "?" }
    static var commit: String? { string("WarpSignCommit") }

    /// "0.2.0 (31)"
    static var versionText: String { "\(version) (\(build))" }

    private static func string(_ key: String) -> String? {
        (Bundle.main.object(forInfoDictionaryKey: key) as? String).flatMap { $0.isEmpty ? nil : $0 }
    }
}
