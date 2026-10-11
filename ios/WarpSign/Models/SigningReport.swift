import Foundation

/// A signed copy's whole record as Markdown, to paste into a bug report or
/// an LLM: options, per component the bundle IDs, capabilities and an
/// entitlements diff with reasons, then profiles, warnings and the log
enum SigningReport {
    static func markdown(app: LibraryApp, copy: SignedCopy, record: SigningRecord) -> String {
        var out: [String] = []
        out.append("# WarpSign signing report: \(app.displayName)")
        out.append("")
        let version = [app.version, app.build.map { "(\($0))" }].compactMap { $0 }.joined(separator: " ")
        out.append("- App: \(app.displayName)\(version.isEmpty ? "" : " \(version)")")
        out.append("- Bundle ID: `\(app.bundleId)` → `\(copy.bundleId)`")
        out.append("- Signed: \(copy.date.formatted(.iso8601))")
        out.append("- Team: \(copy.teamId)")
        out.append("- Certificate: \(copy.certificate)")
        out.append("- PPQ checking: \(copy.ppqCheck ? "on" : "off")")
        if let expires = copy.profileExpires {
            out.append("- Profile expires: \(expires.formatted(.iso8601))")
        }

        out.append("")
        out.append("## Options")
        out.append("")
        out.append(contentsOf: optionLines(record.options, customIcon: record.customIcon))

        if !record.warnings.isEmpty {
            out.append("")
            out.append("## Warnings")
            out.append("")
            out.append(contentsOf: record.warnings.map { "- \($0)" })
        }

        out.append("")
        out.append("## Components")
        let unchanged = record.components.filter { !$0.isPrimary && $0.changes.isEmpty }
        let listed = mostEntitlementsFirst(record.components.filter { $0.isPrimary || !$0.changes.isEmpty }) {
            $0.entitlementsXml
        }
        for component in listed {
            out.append("")
            out.append(contentsOf: componentLines(component))
        }
        if !unchanged.isEmpty {
            out.append("")
            out.append("Also re-signed, with no entitlements: " + unchanged.map { "`\($0.path)`" }.joined(separator: ", "))
        }

        if !record.profiles.isEmpty {
            out.append("")
            out.append("## Profiles")
            out.append("")
            for profile in record.profiles {
                var line = "- `\(path(profile.bundlePath))`: \(profile.name) (`\(profile.bundleId)`)"
                if let expires = profile.expires { line += ", expires \(expires.formatted(.iso8601))" }
                if profile.ppqCheck { line += ", PPQ" }
                out.append(line)
            }
        }

        if !record.steps.isEmpty {
            out.append("")
            out.append("## Steps")
            out.append("")
            out.append("```")
            out.append(record.steps.transcript)
            out.append("```")
        }
        return out.joined(separator: "\n") + "\n"
    }

    private static func path(_ path: String) -> String {
        path.isEmpty ? "(app)" : path
    }

    private static func componentLines(_ component: SavedComponent) -> [String] {
        let names = EntitlementNames(component.names)
        let label = { (key: String) -> String in
            names[key].map { "`\(key)` (\($0.name))" } ?? "`\(key)`"
        }
        var out: [String] = []
        let kind = componentKind(path: component.path, isPrimary: component.isPrimary)
        out.append("### \(path(component.path)): \(kind)")
        out.append("")
        if component.originalBundleId == component.bundleId {
            out.append("- Bundle ID: `\(component.bundleId)`")
        } else {
            out.append("- Bundle ID: `\(component.originalBundleId)` → `\(component.bundleId)`")
        }
        if !component.capabilities.isEmpty {
            let capabilities = component.capabilities.map { capability -> String in
                let name = names.capabilityName(capability)
                return name.map { "\($0) (`\(capability)`)" } ?? "`\(capability)`"
            }
            out.append("- Capabilities: " + capabilities.joined(separator: ", "))
        }
        for change in component.changes {
            let verb: String
            switch change.kind {
            case .added: verb = "added"
            case .removed: verb = "removed"
            case .changed: verb = "changed"
            }
            var line = "- \(label(change.key)) \(verb)"
            if let reason = change.reason { line += ": \(reason)" }
            out.append(line)
        }

        let original = Entitlements.parse(component.originalEntitlementsXml) ?? []
        let signed = Entitlements.parse(component.entitlementsXml) ?? []
        if original.isEmpty && signed.isEmpty {
            out.append("")
            out.append("No entitlements.")
            return out
        }
        out.append("")
        out.append("Entitlements, original (-) to signed (+):")
        out.append("")
        out.append("```diff")
        out.append(contentsOf: diffLines(original: original, signed: signed))
        out.append("```")
        return out
    }

    /// Every key, unchanged ones as context, in key order
    private static func diffLines(original: [PlistEntry], signed: [PlistEntry]) -> [String] {
        var before: [String: PlistValue] = [:]
        for entry in original { before[entry.key] = entry.value }
        var after: [String: PlistValue] = [:]
        for entry in signed { after[entry.key] = entry.value }
        let keys = Set(before.keys).union(after.keys).sorted()

        var out: [String] = []
        for key in keys {
            switch (before[key], after[key]) {
            case let (old?, new?) where old == new:
                out.append(contentsOf: entryLines(key, old, marker: " "))
            case let (old?, new?):
                out.append(contentsOf: entryLines(key, old, marker: "-"))
                out.append(contentsOf: entryLines(key, new, marker: "+"))
            case let (old?, nil):
                out.append(contentsOf: entryLines(key, old, marker: "-"))
            case let (nil, new?):
                out.append(contentsOf: entryLines(key, new, marker: "+"))
            case (nil, nil):
                break
            }
        }
        return out
    }

    private static func entryLines(_ key: String, _ value: PlistValue, marker: String) -> [String] {
        if let text = value.scalarText {
            return ["\(marker) \(key): \(text)"]
        }
        var out = ["\(marker) \(key):"]
        for line in value.lines(depth: 1) {
            out.append(marker + " " + String(repeating: "  ", count: line.depth) + line.text)
        }
        return out
    }

    private static func optionLines(_ options: SigningOptions?, customIcon: Bool) -> [String] {
        guard let options else { return ["The options weren't recorded."] }
        let onOff = { (on: Bool) -> String in on ? "on" : "off" }
        return [
            "- Certificate: \(options.certificate.title)",
            "- Unique identifiers: \(onOff(options.encodeIds))",
            "- Patch IDs in binaries: \(onOff(options.patchIds))",
            "- Keep original bundle ID: \(onOff(options.forceOriginalId))",
            "- Name: \(options.bundleName.isEmpty ? "unchanged" : options.bundleName)",
            "- Icon: \(customIcon ? "replaced" : "unchanged")",
            "- Support all devices: \(onOff(options.patchAllDevices))",
            "- File sharing: \(onOff(options.patchFileSharing))",
            "- ProMotion: \(onOff(options.patchPromotion))",
            "- Fullscreen on iPad: \(onOff(options.patchFullscreen))",
            "- All orientations: \(onOff(options.patchOrientation))",
            "- Game Mode: \(onOff(options.patchGameMode))",
            "- Remove URL schemes: \(onOff(options.removeUrlSchemes))",
            "- Status bar: \(options.statusBar.title)",
            "- Appearance: \(options.interfaceStyle.title)",
            "- Allow debugging: \(onOff(options.patchDebug))",
        ]
    }
}
