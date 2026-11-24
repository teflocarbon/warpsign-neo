import Foundation

/// An app as the App Store's catalog describes it: Apple's public search
/// and lookup service (itunes.apple.com), which needs no Apple ID and no
/// Unicorn. ipatool is only needed to get and download an app.
struct CatalogApp: Codable, Hashable, Identifiable {
    let trackId: Int64
    let bundleId: String
    let trackName: String
    var version: String?
    var sellerName: String?
    var artistName: String?
    var artworkUrl512: String?
    var artworkUrl100: String?
    var screenshotUrls: [String]?
    var ipadScreenshotUrls: [String]?
    var description: String?
    var releaseNotes: String?
    var currentVersionReleaseDate: String?
    var averageUserRating: Double?
    var userRatingCount: Int?
    var primaryGenreName: String?
    var contentAdvisoryRating: String?
    var fileSizeBytes: String?
    var minimumOsVersion: String?
    var price: Double?
    var formattedPrice: String?
    var currency: String?
    var sellerUrl: String?
    var trackViewUrl: String?

    var id: Int64 { trackId }
    var name: String { trackName }
    var developer: String? { sellerName ?? artistName }
    var iconURL: String? { artworkUrl512 ?? artworkUrl100 }
    var isFree: Bool { (price ?? 0) == 0 }
    var size: UInt64? { fileSizeBytes.flatMap(UInt64.init) }
    var releaseDate: Date? { currentVersionReleaseDate.flatMap(SourceDate.parse) }
    /// iPhone screenshots, or the iPad's for iPad-only apps
    var screenshots: [String] {
        let phone = screenshotUrls ?? []
        return phone.isEmpty ? (ipadScreenshotUrls ?? []) : phone
    }

    /// The iOS version it needs, when that's newer than this device's
    var needsNewerSystem: String? {
        guard let minimum = minimumOsVersion else { return nil }
        let system = ProcessInfo.processInfo.operatingSystemVersion
        let current = "\(system.majorVersion).\(system.minorVersion).\(system.patchVersion)"
        return minimum.compare(current, options: .numeric) == .orderedDescending ? minimum : nil
    }

    /// What Get says: Get, or the price
    var getTitle: String {
        isFree ? "Get" : (formattedPrice ?? price.map { String(format: "%.2f", $0) } ?? "Buy")
    }

    /// One the catalog doesn't list (taken off the store, or not in this
    /// country), from what ipatool knows
    init(_ app: StoreAppInfo) {
        trackId = app.id
        bundleId = app.bundleId
        trackName = app.name
        version = app.version.isEmpty ? nil : app.version
        price = app.price
    }
}

enum AppStoreCatalog {
    private struct Results: Decodable {
        let results: [Entry]
    }

    /// Lookups can return other kinds (a developer, for one): keep apps
    private struct Entry: Decodable {
        let app: CatalogApp?

        init(from decoder: Decoder) throws {
            app = try? CatalogApp(from: decoder)
        }
    }

    static func search(_ term: String, country: String, limit: Int = 25) async throws -> [CatalogApp] {
        try await fetch("search", [
            URLQueryItem(name: "term", value: term),
            URLQueryItem(name: "entity", value: "software"),
            URLQueryItem(name: "limit", value: String(limit)),
            URLQueryItem(name: "country", value: country),
        ])
    }

    /// The apps with these App Store IDs that the country's store lists, in
    /// no particular order
    static func lookup(ids: [Int64], country: String) async throws -> [CatalogApp] {
        var found: [CatalogApp] = []
        // The service takes a good number at once; stay well under its limit
        for start in stride(from: 0, to: ids.count, by: 100) {
            let batch = ids[start..<min(start + 100, ids.count)]
            found += try await fetch("lookup", [
                URLQueryItem(name: "id", value: batch.map(String.init).joined(separator: ",")),
                URLQueryItem(name: "country", value: country),
            ])
        }
        return found
    }

    /// The app with this bundle ID, if the country's store lists it
    static func lookup(bundleId: String, country: String) async throws -> CatalogApp? {
        try await fetch("lookup", [
            URLQueryItem(name: "bundleId", value: bundleId),
            URLQueryItem(name: "country", value: country),
        ]).first
    }

    // MARK: Charts

    enum Chart: String, CaseIterable, Identifiable {
        case free = "top-free"
        case paid = "top-paid"

        var id: String { rawValue }
        var title: String {
            switch self {
            case .free: "Top Free"
            case .paid: "Top Paid"
            }
        }
    }

    private struct Feed: Decodable {
        struct Body: Decodable {
            let results: [Item]
        }
        struct Item: Decodable {
            let id: String
        }
        let feed: Body
    }

    /// The country's chart, in order: Apple's public chart feed lists the
    /// IDs (100 at most), and the catalog describes them
    static func chart(_ chart: Chart, country: String) async throws -> [CatalogApp] {
        let url = URL(string: "https://rss.marketingtools.apple.com/api/v2/\(country.lowercased())/apps/\(chart.rawValue)/100/apps.json")!
        let data = try await get(url)
        let ids = try JSONDecoder().decode(Feed.self, from: data).feed.results.compactMap { Int64($0.id) }
        let apps = try await lookup(ids: ids, country: country)
        let byId = Dictionary(apps.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return ids.compactMap { byId[$0] }
    }

    // MARK: What was typed

    /// What a search box holds: words to search for, or one app named by an
    /// App Store link, its App Store ID or its bundle ID
    enum Query: Equatable {
        case term(String)
        case id(Int64)
        case bundleId(String)

        init(_ text: String) {
            let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
            // apps.apple.com/gb/app/name/id123456789, itms-apps://…?id=…
            if text.contains("apple.com") || text.hasPrefix("itms") {
                if let match = text.firstMatch(of: #/id(\d{5,})/#), let id = Int64(match.1) {
                    self = .id(id)
                    return
                }
            }
            if text.wholeMatch(of: #/\d{5,}/#) != nil, let id = Int64(text) {
                self = .id(id)
            } else if text.wholeMatch(of: #/[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)+/#) != nil,
                      text.contains(where: \.isLetter) {
                // Dotted words, as com.example.app. When no app has that
                // bundle ID, they're searched for as words instead.
                self = .bundleId(text)
            } else {
                self = .term(text)
            }
        }
    }

    /// Search for what was typed. One app named by link, ID or bundle ID
    /// comes first; when this country's store hasn't got it, the stores
    /// in `fallbacks` are tried, and `foundIn` says which had it.
    static func find(
        _ text: String, country: String, fallbacks: [String]
    ) async throws -> (apps: [CatalogApp], foundIn: String?) {
        let query = Query(text)
        func one(in country: String) async throws -> CatalogApp? {
            switch query {
            case .id(let id): return try await lookup(ids: [id], country: country).first
            case .bundleId(let bundleId): return try await lookup(bundleId: bundleId, country: country)
            case .term: return nil
            }
        }
        switch query {
        case .term(let term):
            return (try await search(term, country: country), nil)
        case .id, .bundleId:
            if let app = try await one(in: country) {
                var apps = [app]
                if case .bundleId(let words) = query {
                    apps += try await search(words, country: country).filter { $0.id != app.id }
                }
                return (apps, nil)
            }
            for other in fallbacks where other != country {
                if let app = try await one(in: other) { return ([app], other) }
            }
            if case .bundleId(let words) = query {
                return (try await search(words, country: country), nil)
            }
            return ([], nil)
        }
    }

    /// Every country with a store, by code
    static var allCountries: [String] { Array(countries.values) }

    private static func fetch(_ path: String, _ query: [URLQueryItem]) async throws -> [CatalogApp] {
        var components = URLComponents(string: "https://itunes.apple.com/\(path)")!
        components.queryItems = query
        let data = try await get(components.url!)
        return try JSONDecoder().decode(Results.self, from: data).results.compactMap(\.app)
    }

    /// Apple's catalog services now and then answer slowly, or with an
    /// error (403, 429 or 5xx), and a moment later answer fine. Each
    /// request has a time limit, and is tried up to three times.
    private static func get(_ url: URL) async throws -> Data {
        let request = URLRequest(url: url, timeoutInterval: 12)
        var attempt = 0
        while true {
            attempt += 1
            do {
                let (data, response) = try await URLSession.shared.data(for: request)
                let status = (response as? HTTPURLResponse)?.statusCode ?? 200
                if status == 200 { return data }
                if attempt >= 3 || !(status == 403 || status == 429 || status >= 500) {
                    throw CatalogError(status: status)
                }
            } catch let error as URLError
                where attempt < 3 && [.timedOut, .networkConnectionLost, .cannotConnectToHost].contains(error.code) {
                // Tried again below
            }
            try await Task.sleep(for: .milliseconds(attempt == 1 ? 500 : 1500))
        }
    }

    struct CatalogError: LocalizedError {
        let status: Int
        var errorDescription: String? {
            "Apple's App Store catalog isn't answering properly right now (HTTP \(status)). Try again in a moment."
        }
    }

    /// The country code ("US") of a storefront as Apple writes it
    /// ("143441-1,29"), so the catalog shows the store the account uses
    static func country(storefront: String) -> String? {
        let id = storefront
            .split(separator: ",").first?
            .split(separator: "-").first
            .map(String.init) ?? ""
        return countries[id]
    }

    /// The store when signed out: the device's region
    static var deviceCountry: String {
        Locale.current.region?.identifier ?? "US"
    }

    /// Storefront IDs by country, from ipatool's pkg/appstore/storefront.go
    /// (MIT, Copyright (c) 2021 Majd Alfhaily; see NOTICE)
    private static let countries: [String: String] = {
        let table = """
        AE:143481 AG:143540 AI:143538 AL:143575 AM:143524 AO:143564 AR:143505 AT:143445 \
        AU:143460 AZ:143568 BB:143541 BD:143490 BE:143446 BG:143526 BH:143559 BM:143542 \
        BN:143560 BO:143556 BR:143503 BS:143539 BW:143525 BY:143565 BZ:143555 CA:143455 \
        CH:143459 CI:143527 CL:143483 CN:143465 CO:143501 CR:143495 CY:143557 CZ:143489 \
        DE:143443 DK:143458 DM:143545 DO:143508 DZ:143563 EC:143509 EE:143518 EG:143516 \
        ES:143454 FI:143447 FR:143442 GB:143444 GD:143546 GE:143615 GH:143573 GR:143448 \
        GT:143504 GY:143553 HK:143463 HN:143510 HR:143494 HU:143482 ID:143476 IE:143449 \
        IL:143491 IN:143467 IS:143558 IT:143450 IQ:143617 JM:143511 JO:143528 JP:143462 \
        KE:143529 KN:143548 KR:143466 KW:143493 KY:143544 KZ:143517 LB:143497 LC:143549 \
        LI:143522 LK:143486 LT:143520 LU:143451 LV:143519 MD:143523 MG:143531 MK:143530 \
        ML:143532 MN:143592 MO:143515 MS:143547 MT:143521 MU:143533 MV:143488 MX:143468 \
        MY:143473 NE:143534 NG:143561 NI:143512 NL:143452 NO:143457 NP:143484 NZ:143461 \
        OM:143562 PA:143485 PE:143507 PH:143474 PK:143477 PL:143478 PT:143453 PY:143513 \
        QA:143498 RO:143487 RS:143500 RU:143469 SA:143479 SE:143456 SG:143464 SI:143499 \
        SK:143496 SN:143535 SR:143554 SV:143506 TC:143552 TH:143475 TN:143536 TR:143480 \
        TT:143551 TW:143470 TZ:143572 UA:143492 UG:143537 US:143441 UY:143514 UZ:143566 \
        VC:143550 VE:143502 VG:143543 VN:143471 YE:143571 ZA:143472
        """
        var countries: [String: String] = [:]
        for pair in table.split(separator: " ") {
            let parts = pair.split(separator: ":")
            countries[String(parts[1])] = String(parts[0])
        }
        return countries
    }()
}
