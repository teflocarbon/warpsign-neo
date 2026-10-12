import Foundation

/// The signing options form, remembered between signs. Defaults are Python
/// warpsign's.
struct SigningOptions: Codable, Equatable {
    enum StatusBarChoice: String, Codable, CaseIterable, Identifiable {
        case unchanged, hidden, light, dark
        var id: Self { self }
        var title: String {
            switch self {
            case .unchanged: "Unchanged"
            case .hidden: "Hidden"
            case .light: "Light Content"
            case .dark: "Dark Content"
            }
        }
    }

    enum StyleChoice: String, Codable, CaseIterable, Identifiable {
        case automatic, light, dark
        var id: Self { self }
        var title: String {
            switch self {
            case .automatic: "Automatic"
            case .light: "Always Light"
            case .dark: "Always Dark"
            }
        }
    }

    enum CertificateChoice: String, Codable, CaseIterable, Identifiable {
        case development, distribution
        var id: Self { self }
        var type: CertificateType { self == .development ? .development : .distribution }
        var title: String { self == .development ? "Development" : "Distribution (Ad Hoc)" }
    }

    var certificate: CertificateChoice = .development
    var bundleName = ""
    var encodeIds = true
    var patchIds = true
    var forceOriginalId = false
    var patchDebug = false
    var patchAllDevices = false
    var patchFileSharing = false
    var patchPromotion = false
    var patchFullscreen = false
    var patchOrientation = false
    var patchGameMode = false
    var statusBar: StatusBarChoice = .unchanged
    var interfaceStyle: StyleChoice = .automatic
    var removeUrlSchemes = false

    /// For warpsign-core, with a new icon (PNG) if one was chosen
    func settings(icon: Data?) -> SignSettings {
        let name = bundleName.trimmingCharacters(in: .whitespaces)
        return SignSettings(
            bundleName: name.isEmpty ? nil : name,
            icon: icon,
            encodeIds: encodeIds,
            patchIds: patchIds,
            forceOriginalId: forceOriginalId,
            patchDebug: patchDebug,
            patchAllDevices: patchAllDevices,
            patchFileSharing: patchFileSharing,
            patchPromotion: patchPromotion,
            patchFullscreen: patchFullscreen,
            patchOrientation: patchOrientation,
            patchGameMode: patchGameMode,
            statusBar: ffiStatusBar,
            interfaceStyle: ffiInterfaceStyle,
            removeUrlSchemes: removeUrlSchemes,
            pretendNotGranted: DeveloperOptions.pretendNotGranted
        )
    }

    private var ffiStatusBar: StatusBar {
        switch statusBar {
        case .unchanged: return .unchanged
        case .hidden: return .hidden
        case .light: return .light
        case .dark: return .dark
        }
    }

    private var ffiInterfaceStyle: InterfaceStyle {
        switch interfaceStyle {
        case .automatic: return .automatic
        case .light: return .light
        case .dark: return .dark
        }
    }

    private static let key = "signingOptions"

    /// Where a sign of `app` starts: the options it was last signed with,
    /// or for an app never signed, the last options used with what decides
    /// its bundle ID (and its name) back at their defaults, so one app's
    /// choices never carry over to another
    @MainActor
    static func starting(for app: LibraryApp, in library: LibraryStore) -> SigningOptions {
        if let last = library.lastSigningOptions(of: app) { return last }
        var options = saved
        let defaults = SigningOptions()
        options.encodeIds = defaults.encodeIds
        options.patchIds = defaults.patchIds
        options.forceOriginalId = defaults.forceOriginalId
        options.bundleName = defaults.bundleName
        return options
    }

    /// The last options used, without the per-app name
    static var saved: SigningOptions {
        guard let data = UserDefaults.standard.data(forKey: key),
              let options = try? JSONDecoder().decode(SigningOptions.self, from: data)
        else { return SigningOptions() }
        return options
    }

    func save() {
        var options = self
        options.bundleName = ""
        if let data = try? JSONEncoder().encode(options) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }
}
