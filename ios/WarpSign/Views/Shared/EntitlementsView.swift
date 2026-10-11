import SwiftUI
import UIKit

/// SF Symbols for entitlements: the portal's own (`sfSymbolIcon`) when it
/// gives one this iOS has, else picked by hand
enum EntitlementSymbol {
    static func name(key: String, capability: String?, portal: String? = nil) -> String {
        // Newer symbols may not exist on older iOS
        if let portal, UIImage(systemName: portal) != nil { return portal }
        if let capability, let symbol = byCapability[capability] { return symbol }
        return byKey[key] ?? "checkmark.seal"
    }

    /// Portal capability ID → symbol
    private static let byCapability: [String: String] = [
        "PUSH_NOTIFICATIONS": "bell.badge",
        "APP_GROUPS": "square.stack.3d.up",
        "ICLOUD": "icloud",
        "ASSOCIATED_DOMAINS": "link",
        "IN_APP_PAYMENTS": "creditcard",
        "HEALTHKIT": "heart",
        "HOMEKIT": "house",
        "APPLE_ID_AUTH": "apple.logo",
        "NFC_TAG_READING": "wave.3.right",
        "GAME_CENTER": "gamecontroller",
        "SIRIKIT": "mic",
        "WALLET": "wallet.pass",
        "NETWORK_EXTENSIONS": "network",
        "PERSONAL_VPN": "lock.shield",
        "ACCESS_WIFI_INFORMATION": "wifi",
        "HOT_SPOT": "personalhotspot",
        "MULTIPATH": "arrow.triangle.branch",
        "DATA_PROTECTION": "lock.doc",
        "INTER_APP_AUDIO": "speaker.wave.2",
        "CLASSKIT": "graduationcap",
        "AUTOFILL_CREDENTIAL_PROVIDER": "person.badge.key",
        "USERNOTIFICATIONS_TIMESENSITIVE": "clock",
        "USERNOTIFICATIONS_COMMUNICATION": "message",
        "WEATHERKIT": "cloud.sun",
        "FAMILY_CONTROLS": "person.2",
        "MAPS": "map",
        "APP_ATTEST": "checkmark.shield",
        "FONT_INSTALLATION": "textformat",
        "COREMEDIA_HLS_LOW_LATENCY": "play.rectangle",
        "PUSH_TO_TALK": "antenna.radiowaves.left.and.right",
        "GROUP_ACTIVITIES": "shareplay",
        "EXTENDED_VIRTUAL_ADDRESSING": "memorychip",
        "INCREASED_MEMORY_LIMIT": "memorychip",
    ]

    /// For keys that aren't capabilities, and common ones when the
    /// capabilities couldn't be fetched
    private static let byKey: [String: String] = [
        "application-identifier": "app",
        "com.apple.developer.team-identifier": "person.2",
        "keychain-access-groups": "key",
        "get-task-allow": "ladybug",
        "beta-reports-active": "airplane",
        "aps-environment": "bell.badge",
        "com.apple.security.application-groups": "square.stack.3d.up",
        "com.apple.developer.icloud-container-identifiers": "icloud",
        "com.apple.developer.ubiquity-container-identifiers": "icloud",
        "com.apple.developer.icloud-services": "icloud",
        "com.apple.developer.ubiquity-kvstore-identifier": "icloud",
        "com.apple.developer.associated-domains": "link",
        "com.apple.developer.healthkit": "heart",
        "com.apple.developer.applesignin": "apple.logo",
    ]
}

/// One entitlement: its name (when the portal gives one), key and value.
/// Long-press to copy.
struct EntitlementRow: View {
    let entry: PlistEntry
    let name: EntitlementName?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            EntitlementIcon(key: entry.key, name: name)
            VStack(alignment: .leading, spacing: 3) {
                if let name {
                    Text(name.name).font(.subheadline.weight(.semibold))
                }
                Text(entry.key)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                PlistValueText(value: entry.value)
            }
        }
        .contextMenu {
            Button("Copy Value", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = entry.value.copyText
            }
            Button("Copy Key", systemImage: "key") {
                UIPasteboard.general.string = entry.key
            }
        }
    }
}

struct EntitlementIcon: View {
    let key: String
    let name: EntitlementName?

    var body: some View {
        Image(systemName: EntitlementSymbol.name(key: key, capability: name?.capability, portal: name?.symbol))
            .foregroundStyle(.tint)
            .frame(width: 24)
    }
}

/// A plist value: scalars on one line, arrays one item per line,
/// dictionaries as "key: value"
struct PlistValueText: View {
    let value: PlistValue

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(value.lines().enumerated()), id: \.offset) { _, line in
                Text(line.text)
                    .font(.callout.monospaced())
                    .padding(.leading, CGFloat(line.depth) * 14)
            }
        }
    }
}

/// Rows for every entitlement in a dictionary
struct EntitlementList: View {
    let entries: [PlistEntry]?
    let names: EntitlementNames

    var body: some View {
        if let entries {
            if entries.isEmpty {
                Text("No entitlements").foregroundStyle(.secondary)
            } else {
                ForEach(entries) { entry in
                    EntitlementRow(entry: entry, name: names[entry.key])
                }
            }
        } else {
            ErrorRow(message: "Couldn't read the entitlements.")
        }
    }
}

/// An XML plist, selectable and copyable
struct RawPlistView: View {
    let title: String
    let xml: String

    var body: some View {
        ScrollView {
            Text(xml)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Copy", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = xml
                }
            }
        }
    }
}

/// "App", "Extension" or "Framework" for a component path
func componentKind(path: String, isPrimary: Bool) -> String {
    if path.isEmpty { return "App" }
    if !isPrimary { return path.hasSuffix(".dylib") ? "Library" : "Framework" }
    return path.hasSuffix(".app") ? "App" : "Extension"
}

/// A component's name for titles: "App" or its bundle's file name
func componentTitle(path: String) -> String {
    path.isEmpty ? "App" : (path as NSString).lastPathComponent
}

// MARK: Before signing

/// One component of an inspected app and the entitlements it has
struct ComponentEntitlementsView: View {
    let component: Component
    let names: EntitlementNames
    private let entries: [PlistEntry]?

    init(component: Component, names: EntitlementNames) {
        self.component = component
        self.names = names
        entries = Entitlements.parse(component.entitlementsXml)
    }

    var body: some View {
        List {
            Section {
                LabeledContent("Kind", value: componentKind(path: component.path, isPrimary: component.isPrimary))
                if !component.path.isEmpty {
                    LabeledContent("Path") {
                        Text(component.path).font(.caption.monospaced()).textSelection(.enabled)
                    }
                }
                LabeledContent("Bundle ID") {
                    Text(component.bundleId).font(.caption.monospaced()).textSelection(.enabled)
                }
            }
            Section {
                EntitlementList(entries: entries, names: names)
            } header: {
                Text("Entitlements")
            } footer: {
                if !component.isPrimary {
                    Text("Frameworks and libraries are re-signed without entitlements.")
                } else {
                    Text("What it's signed with now, before WarpSign signs it. Long-press one to copy it.")
                }
            }
            Section {
                NavigationLink {
                    RawPlistView(title: "Entitlements", xml: component.entitlementsXml)
                } label: {
                    Label("Raw Plist", systemImage: "chevron.left.forwardslash.chevron.right")
                }
            }
        }
        .navigationTitle(componentTitle(path: component.path))
        .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: After signing

/// One component of a signed copy: what it was signed with, compared with
/// what it had before
struct SignedComponentView: View {
    enum Tab: String, CaseIterable, Identifiable {
        case changes = "Changes"
        case signed = "Signed"
        case original = "Original"
        var id: Self { self }
    }

    let component: SavedComponent
    private let original: [PlistEntry]?
    private let signed: [PlistEntry]?
    private let names: EntitlementNames
    @State private var tab = Tab.changes

    init(component: SavedComponent) {
        self.component = component
        original = Entitlements.parse(component.originalEntitlementsXml)
        signed = Entitlements.parse(component.entitlementsXml)
        names = EntitlementNames(component.names)
    }

    /// This component's own entitlement keys that `capability` grants, as
    /// they appear in its plists (e.g. com.apple.security.application-groups)
    private func keys(for capability: String) -> [String] {
        var keys = Set<String>()
        for entry in (signed ?? []) + (original ?? []) where names[entry.key]?.capability == capability {
            keys.insert(entry.key)
        }
        return keys.sorted()
    }

    private func capabilityRow(_ capability: String) -> some View {
        let granted = keys(for: capability)
        let symbol = granted.first.flatMap { names[$0]?.symbol }
        return Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(names.capabilityName(capability) ?? capability)
                ForEach(granted, id: \.self) { key in
                    Text(key)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                }
                Text(capability)
                    .font(.caption2.monospaced())
                    .foregroundStyle(.secondary)
            }
        } icon: {
            Image(systemName: EntitlementSymbol.name(key: granted.first ?? "", capability: capability, portal: symbol))
                .foregroundStyle(.tint)
        }
    }

    var body: some View {
        List {
            Section {
                LabeledContent("Kind", value: componentKind(path: component.path, isPrimary: component.isPrimary))
                if component.bundleId == component.originalBundleId {
                    LabeledContent("Bundle ID") {
                        Text(component.bundleId).font(.caption.monospaced()).textSelection(.enabled)
                    }
                } else {
                    LabeledContent("Was") {
                        Text(component.originalBundleId).font(.caption.monospaced()).textSelection(.enabled)
                    }
                    LabeledContent("Now") {
                        Text(component.bundleId).font(.caption.monospaced()).textSelection(.enabled)
                    }
                }
            } header: {
                Text("Bundle ID")
            }

            if !component.capabilities.isEmpty {
                Section {
                    ForEach(component.capabilities, id: \.self) { capability in
                        capabilityRow(capability)
                    }
                } header: {
                    Text("Capabilities")
                } footer: {
                    Text("Enabled on its App ID on the Developer Portal.")
                }
            }

            Section {
                Picker("Show", selection: $tab) {
                    ForEach(Tab.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            }

            switch tab {
            case .changes:
                Section {
                    if component.changes.isEmpty {
                        Text(component.isPrimary
                             ? "Signed with exactly the entitlements it had."
                             : "No entitlements, before or after.")
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(component.changes, id: \.key) { change in
                            EntitlementChangeRow(
                                change: change,
                                before: value(change.key, in: original),
                                after: value(change.key, in: signed),
                                name: names[change.key]
                            )
                        }
                    }
                } footer: {
                    if !component.changes.isEmpty {
                        Text("Unchanged entitlements are under Signed.")
                    }
                }
            case .signed:
                Section {
                    EntitlementList(entries: signed, names: names)
                } footer: {
                    Text(component.isPrimary
                         ? "Exactly what's in its signature. Long-press one to copy it."
                         : "Frameworks and libraries are signed without entitlements.")
                }
                Section {
                    NavigationLink {
                        RawPlistView(title: "Signed Entitlements", xml: component.entitlementsXml)
                    } label: {
                        Label("Raw Plist", systemImage: "chevron.left.forwardslash.chevron.right")
                    }
                }
            case .original:
                Section {
                    EntitlementList(entries: original, names: names)
                } footer: {
                    Text("What it was signed with before WarpSign signed it.")
                }
                Section {
                    NavigationLink {
                        RawPlistView(title: "Original Entitlements", xml: component.originalEntitlementsXml)
                    } label: {
                        Label("Raw Plist", systemImage: "chevron.left.forwardslash.chevron.right")
                    }
                }
            }
        }
        .navigationTitle(componentTitle(path: component.path))
        .navigationBarTitleDisplayMode(.inline)
    }

    private func value(_ key: String, in entries: [PlistEntry]?) -> PlistValue? {
        entries?.first { $0.key == key }?.value
    }
}

/// An entitlement added, removed or changed by signing, and why
struct EntitlementChangeRow: View {
    let change: SavedChange
    let before: PlistValue?
    let after: PlistValue?
    let name: EntitlementName?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                EntitlementIcon(key: change.key, name: name)
                VStack(alignment: .leading, spacing: 3) {
                    HStack {
                        Text(name?.name ?? change.key)
                            .font(name == nil ? Font.caption.monospaced() : Font.subheadline.weight(.semibold))
                        Spacer()
                        Badge(title, color: color)
                    }
                    if name != nil {
                        Text(change.key)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                    }
                    if let reason = change.reason {
                        Text(reason)
                            .font(.caption)
                            .foregroundStyle(color)
                    }
                    switch change.kind {
                    case .added:
                        if let after { PlistValueText(value: after) }
                    case .removed:
                        if let before {
                            PlistValueText(value: before)
                                .foregroundStyle(.secondary)
                                .strikethrough()
                        }
                    case .changed:
                        if let before {
                            Text("Before").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                            PlistValueText(value: before).foregroundStyle(.secondary)
                        }
                        if let after {
                            Text("After").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                            PlistValueText(value: after)
                        }
                    }
                }
            }
        }
        .contextMenu {
            Button("Copy Key", systemImage: "key") {
                UIPasteboard.general.string = change.key
            }
            if let after {
                Button("Copy Signed Value", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = after.copyText
                }
            }
            if let before {
                Button("Copy Original Value", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = before.copyText
                }
            }
        }
    }

    private var title: String {
        switch change.kind {
        case .added: return "Added"
        case .removed: return "Removed"
        case .changed: return "Changed"
        }
    }

    private var color: Color {
        switch change.kind {
        case .added: return .green
        case .removed: return .red
        case .changed: return .orange
        }
    }
}

/// A signed component in a list: its path, new bundle ID and what changed
struct SignedComponentRow: View {
    let component: SavedComponent
    private let count: Int?

    init(component: SavedComponent) {
        self.component = component
        count = Entitlements.parse(component.entitlementsXml)?.count
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(component.path.isEmpty ? "App" : component.path)
            Text(component.bundleId)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(summary)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var summary: String {
        let entitlements: String
        switch count {
        case nil: entitlements = "Entitlements unreadable"
        case 1?: entitlements = "1 entitlement"
        case let count?: entitlements = "\(count) entitlements"
        }
        return "\(entitlements) · \(changeSummary(component.changes))"
    }
}

/// The components worth listing: apps and extensions, and frameworks that
/// lost entitlements
func listedComponents(_ components: [SavedComponent]) -> [SavedComponent] {
    mostEntitlementsFirst(components.filter { $0.isPrimary || !$0.changes.isEmpty }) {
        $0.entitlementsXml
    }
}

/// Components with the most entitlements first, as those usually matter
/// most; ties keep their order
func mostEntitlementsFirst<T>(_ components: [T], entitlementsXml: (T) -> String) -> [T] {
    struct Ranked {
        let index: Int
        let count: Int
    }
    var ranked: [Ranked] = []
    for (index, component) in components.enumerated() {
        let count = Entitlements.parse(entitlementsXml(component))?.count ?? 0
        ranked.append(Ranked(index: index, count: count))
    }
    ranked.sort { first, second in
        if first.count != second.count { return first.count > second.count }
        return first.index < second.index
    }
    return ranked.map { components[$0.index] }
}

/// A one-line summary of a component's changes, e.g. "2 added, 1 removed"
func changeSummary(_ changes: [SavedChange]) -> String {
    let counts: [(SavedChange.Kind, String)] = [(.added, "added"), (.removed, "removed"), (.changed, "changed")]
    let parts = counts.compactMap { pair -> String? in
        let count = changes.filter { $0.kind == pair.0 }.count
        return count == 0 ? nil : "\(count) \(pair.1)"
    }
    return parts.isEmpty ? "No changes" : parts.joined(separator: ", ")
}
