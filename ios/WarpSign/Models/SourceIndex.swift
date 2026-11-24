import Foundation

/// A source's apps as lists show them: one listing per app, however many
/// builds of it the source has, and the text to search them by.
///
/// Big sources list the same app many times over: tweaked builds of
/// YouTube, each its own entry with YouTube's bundle ID. Grouped by bundle
/// ID, YouTube is listed once, and its page lists the builds. Builds that
/// share a bundle ID but not a name (a source that gives all its apps one
/// ID) aren't the same app, and stay apart.
///
/// Built once per fetch, off the main thread: a source can have tens of
/// thousands of apps.
struct SourceIndex: Sendable {
    /// Featured apps first, as the source asks, then in the source's order
    let listings: [SourceListing]
    /// How many apps the source lists, builds and all
    let buildCount: Int
    /// Told apart from the index of a source fetched again
    let id = UUID()

    init(_ source: Source) {
        var order: [String] = []
        var byBundle: [String: [SourceApp]] = [:]
        for app in source.apps {
            if byBundle[app.bundleId] == nil { order.append(app.bundleId) }
            byBundle[app.bundleId, default: []].append(app)
        }
        let featured = Set(source.featuredApps)
        let ordered = order.filter { featured.contains($0) } + order.filter { !featured.contains($0) }
        listings = ordered.flatMap { Self.listings(of: byBundle[$0] ?? []) }
        buildCount = source.apps.count
    }

    /// The listings matching `query` (any build's name, subtitle,
    /// developer or bundle ID): names starting with it first, then names
    /// with it, then the rest, each in the source's order
    func search(_ query: String) -> [SourceListing] {
        let query = SourceListing.fold(query.trimmingCharacters(in: .whitespacesAndNewlines))
        guard !query.isEmpty else { return listings }
        var tiers: [[SourceListing]] = [[], [], []]
        for listing in listings {
            if let rank = listing.rank(folded: query) {
                tiers[rank].append(listing)
            }
        }
        return tiers.flatMap { $0 }
    }

    /// One bundle ID's apps as listings: one, unless they're different
    /// apps sharing the ID
    private static func listings(of apps: [SourceApp]) -> [SourceListing] {
        // The same entry twice over
        var seen = Set<String>()
        let unique = apps.filter { seen.insert($0.name + "\n" + $0.versions[0].downloadUrl).inserted }
        guard let first = unique.first else { return [] }
        guard unique.count > 1 else { return [SourceListing(name: first.name, builds: unique)] }

        let names = unique.map(\.name)
        var distinct: [String: String] = [:]
        for name in names where distinct[name] == nil { distinct[name] = SourceListing.fold(name) }

        // Nearly every build named differently, many of them: different
        // apps under one ID (one source gives its 291 apps the same ID).
        // Tweaked builds of one app are named alike more often than that.
        guard unique.count >= 10 && distinct.count * 10 >= unique.count * 9 else {
            return [SourceListing(name: name(of: names, distinct: distinct, bundleId: unique[0].bundleId), builds: unique)]
        }
        var nameOrder: [String] = []
        var byName: [String: [SourceApp]] = [:]
        for app in unique {
            if byName[app.name] == nil { nameOrder.append(app.name) }
            byName[app.name, default: []].append(app)
        }
        return nameOrder.map { SourceListing(name: $0, builds: byName[$0] ?? [], separate: true) }
    }

    /// The app's own name, of its builds' `names`: best effort. Tweaked
    /// builds add to the name ("YouTubeOLED", "YouTube (YTKACE 1.1.8)",
    /// "EeveeSpotify"), so it's the name in the bundle ID if one is
    /// ("WhatsApp" for net.whatsapp.WhatsApp, not "Watusi3"), then the one
    /// the most names have as a word, then the shortest. Checked against a
    /// source of 17,000 apps in November 2026.
    private static func name(of names: [String], distinct: [String: String], bundleId: String) -> String {
        let folded = names.map(SourceListing.fold)
        let bundle = SourceListing.fold(bundleId)
        var best: (name: String, inId: Bool, count: Int)?
        for (name, foldedName) in distinct {
            let inId = foldedName.count >= 3 && bundle.contains(foldedName.replacingOccurrences(of: " ", with: ""))
            let count = folded.reduce(0) { $0 + (hasWord(foldedName, in: $1) ? 1 : 0) }
            if let current = best {
                let better = inId != current.inId ? inId
                    : count != current.count ? count > current.count
                    : name.count != current.name.count ? name.count < current.name.count
                    : name < current.name
                guard better else { continue }
            }
            best = (name, inId, count)
        }
        return best?.name ?? names[0]
    }

    /// Whether `text` has `word` at the start of a word ("x" in "x (nfb)",
    /// not in "bhx")
    private static func hasWord(_ word: String, in text: String) -> Bool {
        var searchFrom = text.startIndex
        while let range = text.range(of: word, range: searchFrom..<text.endIndex) {
            guard range.lowerBound > text.startIndex else { return true }
            let before = text[text.index(before: range.lowerBound)]
            if !before.isLetter && !before.isNumber { return true }
            searchFrom = text.index(after: range.lowerBound)
        }
        return false
    }
}

/// An app in a source, with every build of it the source lists
struct SourceListing: Identifiable, Sendable {
    let id: String
    let bundleId: String
    let name: String
    /// Best first: highest version, then newest. Best effort: versions are
    /// compared number by number, and sources write them as they like.
    let builds: [SourceApp]
    /// What the listing shows: the best build with the listing's name, or
    /// the best build
    let app: SourceApp
    /// How many people or groups made the builds
    let developerCount: Int
    private let foldedName: String
    /// Every build's name, subtitle and developer, and the bundle ID,
    /// folded (fold)
    private let searchText: String

    /// `separate`: one of several apps sharing a bundle ID, told apart by
    /// name
    init(name: String, builds: [SourceApp], separate: Bool = false) {
        let sorted = builds.enumerated().sorted { a, b in
            Self.better(a.element, than: b.element) ?? (a.offset < b.offset)
        }.map(\.element)
        let app = sorted.first { $0.name == name } ?? sorted[0]
        self.bundleId = app.bundleId
        self.id = separate ? app.bundleId + "\n" + name : app.bundleId
        self.name = name
        self.builds = sorted
        self.app = app
        self.developerCount = Set(builds.compactMap(\.developerName)).count
        self.foldedName = Self.fold(name)
        var fields = Set<String>([app.bundleId])
        for build in builds {
            fields.insert(build.name)
            if let subtitle = build.subtitle { fields.insert(subtitle) }
            if let developer = build.developerName { fields.insert(developer) }
        }
        self.searchText = Self.fold(fields.sorted().joined(separator: "\n"))
    }

    /// 0: the name starts with `query`, 1: the name has it, 2: something
    /// else does; nil: nothing does. `query` is folded.
    func rank(folded query: String) -> Int? {
        if foldedName.hasPrefix(query) { return 0 }
        if foldedName.contains(query) { return 1 }
        if searchText.contains(query) { return 2 }
        return nil
    }

    /// The builds matching `query`, as search does, all when it's empty
    func builds(matching query: String) -> [SourceApp] {
        let query = Self.fold(query.trimmingCharacters(in: .whitespacesAndNewlines))
        guard !query.isEmpty else { return builds }
        return builds.filter { build in
            [build.name, build.subtitle, build.developerName, build.versions[0].version]
                .compactMap { $0 }
                .contains { Self.fold($0).contains(query) }
        }
    }

    /// Case, accents and widths dropped, to compare as search does
    static func fold(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
    }

    /// Whether `a` goes before `b`: the higher version, then the newer;
    /// nil when neither says
    private static func better(_ a: SourceApp, than b: SourceApp) -> Bool? {
        let versionA = a.versions[0].version, versionB = b.versions[0].version
        switch versionA.compare(versionB, options: .numeric) {
        case .orderedDescending: return true
        case .orderedAscending: return false
        case .orderedSame: break
        }
        // ISO 8601 dates, which most sources use, sort as text
        let dateA = a.versions[0].date ?? "", dateB = b.versions[0].date ?? ""
        return dateA == dateB ? nil : dateA > dateB
    }
}
