import Foundation

// MARK: Plist values

/// A value from an entitlements plist, for showing
enum PlistValue: Hashable, Sendable {
    case bool(Bool)
    case integer(Int64)
    case real(Double)
    case string(String)
    case date(Date)
    /// Byte count
    case data(Int)
    case array([PlistValue])
    case dictionary([PlistEntry])

    /// From what PropertyListSerialization returns
    init(_ object: Any) {
        if let number = object as? NSNumber {
            if CFGetTypeID(number) == CFBooleanGetTypeID() {
                self = .bool(number.boolValue)
            } else if CFNumberIsFloatType(number as CFNumber) {
                self = .real(number.doubleValue)
            } else {
                self = .integer(number.int64Value)
            }
        } else if let string = object as? String {
            self = .string(string)
        } else if let date = object as? Date {
            self = .date(date)
        } else if let data = object as? Data {
            self = .data(data.count)
        } else if let array = object as? [Any] {
            self = .array(array.map { PlistValue($0) })
        } else if let dictionary = object as? [String: Any] {
            self = .dictionary(PlistEntry.sorted(dictionary))
        } else {
            self = .string(String(describing: object))
        }
    }

    /// The value on one line, for anything but arrays and dictionaries
    var scalarText: String? {
        switch self {
        case .bool(let value): return value ? "true" : "false"
        case .integer(let value): return String(value)
        case .real(let value): return String(value)
        case .string(let value): return value
        case .date(let value): return value.formatted(.iso8601)
        case .data(let count): return "<\(count) bytes>"
        case .array, .dictionary: return nil
        }
    }

    /// Lines to show: an array's items one per line, a dictionary's
    /// entries as "key: value", nested values indented
    func lines(depth: Int = 0) -> [PlistLine] {
        switch self {
        case .array(let items):
            if items.isEmpty { return [PlistLine(depth: depth, text: "(empty list)")] }
            return items.flatMap { item -> [PlistLine] in
                if let text = item.scalarText { return [PlistLine(depth: depth, text: text)] }
                return item.lines(depth: depth + 1)
            }
        case .dictionary(let entries):
            if entries.isEmpty { return [PlistLine(depth: depth, text: "(empty dictionary)")] }
            return entries.flatMap { entry -> [PlistLine] in
                if let text = entry.value.scalarText {
                    return [PlistLine(depth: depth, text: "\(entry.key): \(text)")]
                }
                return [PlistLine(depth: depth, text: "\(entry.key):")] + entry.value.lines(depth: depth + 1)
            }
        default:
            return [PlistLine(depth: depth, text: scalarText ?? "")]
        }
    }

    /// For the clipboard
    var copyText: String {
        lines()
            .map { String(repeating: "  ", count: $0.depth) + $0.text }
            .joined(separator: "\n")
    }
}

struct PlistLine: Hashable, Sendable {
    let depth: Int
    let text: String
}

/// A key and its value in a plist dictionary
struct PlistEntry: Hashable, Identifiable, Sendable {
    let key: String
    let value: PlistValue

    var id: String { key }

    /// Entries sorted by key
    static func sorted(_ dictionary: [String: Any]) -> [PlistEntry] {
        dictionary
            .map { PlistEntry(key: $0.key, value: PlistValue($0.value)) }
            .sorted { $0.key < $1.key }
    }
}

enum Entitlements {
    /// The entries of an entitlements XML plist (as warpsign-core gives
    /// them), sorted by key; nil if it can't be read
    static func parse(_ xml: String) -> [PlistEntry]? {
        guard let data = xml.data(using: .utf8),
              let dictionary = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
        else { return nil }
        return PlistEntry.sorted(dictionary)
    }
}

// MARK: Names

/// What an entitlement key is called, e.g. "Push Notifications" for
/// aps-environment, from the Developer Portal's capabilities
struct EntitlementName: Codable, Hashable, Sendable {
    var key: String
    var name: String
    var description: String?
    /// The portal capability that grants it, e.g. "PUSH_NOTIFICATIONS"
    var capability: String?

    init(key: String, name: String, description: String? = nil, capability: String? = nil) {
        self.key = key
        self.name = name
        self.description = description
        self.capability = capability
    }

    init(_ info: EntitlementNameInfo) {
        self.init(key: info.key, name: info.name, description: info.description, capability: info.capability)
    }
}

/// Entitlement names by key
struct EntitlementNames: Sendable {
    private var byKey: [String: EntitlementName] = [:]

    init(_ names: [EntitlementName] = []) {
        for name in names where byKey[name.key] == nil {
            byKey[name.key] = name
        }
    }

    subscript(key: String) -> EntitlementName? { byKey[key] }

    /// The name of a portal capability, from any key it grants
    func capabilityName(_ capability: String) -> String? {
        byKey.values.first { $0.capability == capability }?.name
    }

    /// From a capabilities response (nil: only keys that are never
    /// capabilities get names)
    static func from(capabilitiesJson json: String?) -> EntitlementNames {
        EntitlementNames(entitlementNames(capabilitiesJson: json).map { EntitlementName($0) })
    }
}

/// The team's last capabilities response, which names entitlements, for
/// when the portal can't be reached. Application Support/Caches.
enum CapabilitiesCache {
    static func url(team: String) -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let safe = team.filter { $0.isASCII && ($0.isLetter || $0.isNumber) }
        return base
            .appendingPathComponent("Caches", isDirectory: true)
            .appendingPathComponent("capabilities-\(safe).json")
    }

    static func load(team: String) -> String? {
        guard let data = try? Data(contentsOf: url(team: team)) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    static func save(_ json: String, team: String) {
        let url = url(team: team)
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? Data(json.utf8).write(to: url, options: .atomic)
    }
}

// MARK: Signing records

/// How a signed copy was made: its options, what each component was
/// signed with and the steps it went through. Kept beside the signed IPA
/// as signing.json.
struct SigningRecord: Codable {
    /// nil if it couldn't be read
    var options: SigningOptions?
    var customIcon: Bool
    var warnings: [String]
    var profiles: [SavedProfile]
    var components: [SavedComponent]
    var steps: [SavedStep]

    init(
        options: SigningOptions?,
        customIcon: Bool,
        warnings: [String],
        profiles: [SavedProfile],
        components: [SavedComponent],
        steps: [SavedStep]
    ) {
        self.options = options
        self.customIcon = customIcon
        self.warnings = warnings
        self.profiles = profiles
        self.components = components
        self.steps = steps
    }

    private enum CodingKeys: String, CodingKey {
        case options, customIcon, warnings, profiles, components, steps
    }

    /// Missing fields (from older or newer versions) read as empty
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        options = try? container.decodeIfPresent(SigningOptions.self, forKey: .options)
        customIcon = (try? container.decodeIfPresent(Bool.self, forKey: .customIcon)) ?? false
        warnings = (try? container.decodeIfPresent([String].self, forKey: .warnings)) ?? []
        profiles = (try? container.decodeIfPresent([SavedProfile].self, forKey: .profiles)) ?? []
        components = (try? container.decodeIfPresent([SavedComponent].self, forKey: .components)) ?? []
        steps = (try? container.decodeIfPresent([SavedStep].self, forKey: .steps)) ?? []
    }

    /// From a finished sign
    init(result: SignResult, options: SigningOptions, customIcon: Bool, steps: [StepState]) {
        self.init(
            options: options,
            customIcon: customIcon,
            warnings: result.warnings,
            profiles: result.profiles.map { SavedProfile($0) },
            components: result.components.map { SavedComponent($0) },
            steps: steps.map { SavedStep($0) }
        )
    }
}

struct SavedProfile: Codable, Hashable {
    var bundlePath: String
    var bundleId: String
    var name: String
    var ppqCheck: Bool
    var expires: Date?

    init(bundlePath: String, bundleId: String, name: String, ppqCheck: Bool, expires: Date?) {
        self.bundlePath = bundlePath
        self.bundleId = bundleId
        self.name = name
        self.ppqCheck = ppqCheck
        self.expires = expires
    }

    init(_ profile: SignedProfileInfo) {
        self.init(
            bundlePath: profile.bundlePath,
            bundleId: profile.bundleId,
            name: profile.name,
            ppqCheck: profile.ppqCheck,
            expires: profile.expiresAt.map(Date.init(unixTime:))
        )
    }
}

/// What one component was signed with
struct SavedComponent: Codable, Hashable, Identifiable {
    var path: String
    var isPrimary: Bool
    var originalBundleId: String
    var bundleId: String
    /// XML plists
    var originalEntitlementsXml: String
    var entitlementsXml: String
    /// Portal capability IDs enabled on its App ID
    var capabilities: [String]
    var changes: [SavedChange]
    /// Names for its keys, from the portal when it was signed
    var names: [EntitlementName]

    var id: String { path }

    init(
        path: String,
        isPrimary: Bool,
        originalBundleId: String,
        bundleId: String,
        originalEntitlementsXml: String,
        entitlementsXml: String,
        capabilities: [String],
        changes: [SavedChange],
        names: [EntitlementName]
    ) {
        self.path = path
        self.isPrimary = isPrimary
        self.originalBundleId = originalBundleId
        self.bundleId = bundleId
        self.originalEntitlementsXml = originalEntitlementsXml
        self.entitlementsXml = entitlementsXml
        self.capabilities = capabilities
        self.changes = changes
        self.names = names
    }

    init(_ component: SignedComponentInfo) {
        self.init(
            path: component.path,
            isPrimary: component.isPrimary,
            originalBundleId: component.originalBundleId,
            bundleId: component.bundleId,
            originalEntitlementsXml: component.originalEntitlementsXml,
            entitlementsXml: component.entitlementsXml,
            capabilities: component.capabilities,
            changes: component.changes.map { SavedChange($0) },
            names: component.entitlementNames.map { EntitlementName($0) }
        )
    }
}

/// An entitlement added, removed or changed by signing
struct SavedChange: Codable, Hashable {
    enum Kind: String, Codable {
        case added, removed, changed
    }

    var key: String
    var kind: Kind
    /// Why, when WarpSign knows
    var reason: String?

    init(key: String, kind: Kind, reason: String?) {
        self.key = key
        self.kind = kind
        self.reason = reason
    }

    init(_ change: EntitlementChangeInfo) {
        let kind: Kind
        switch change.kind {
        case .added: kind = .added
        case .removed: kind = .removed
        case .changed: kind = .changed
        }
        self.init(key: change.key, kind: kind, reason: change.reason)
    }
}

/// A step of a finished operation, with its log
struct SavedStep: Codable {
    var title: String
    var status: StepState.Status
    var log: [String]

    init(title: String, status: StepState.Status, log: [String]) {
        self.title = title
        self.status = status
        self.log = log
    }

    init(_ step: StepState) {
        self.init(title: step.title, status: step.status, log: step.log)
    }
}

extension Array where Element == SavedStep {
    /// Plain-text log of every step, for copying into a bug report
    var transcript: String {
        map { StepState.transcript(title: $0.title, status: $0.status, log: $0.log) }
            .joined(separator: "\n")
    }
}
