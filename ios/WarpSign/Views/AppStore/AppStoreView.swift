import SwiftUI

/// The App Store tab: search the store, browse its charts and the apps the
/// account has got, and get apps into the library to sign. Browsing needs
/// no Apple ID; getting apps does, through ipatool.
///
/// A row of chips under the search field (which store, and what to show),
/// then one list: that, or the search's results while there's a search,
/// or recent searches while the search field is empty and in use.
struct AppStoreView: View {
    @Environment(AccountStore.self) private var account
    @State private var shelf = StoreShelf.free
    @State private var term = ""
    @State private var searchedTerm = ""
    @State private var results: [CatalogApp] = []
    /// The store an app named by link or ID was found in, when not this one
    @State private var foundIn: String?
    @State private var searching = false
    @State private var searchError: String?
    @State private var shelfError: String?
    @State private var signingIn = false
    @State private var showingAccount = false
    /// The list's side margin, which its cards and the chips line up with
    @State private var margin: CGFloat = 20
    /// Go's report of the last crash, once ipatool has started
    @State private var lastCrash: String?

    private var store: AppStoreModel { .shared }

    var body: some View {
        NavigationStack {
            Group {
                if AppStoreEngine.isAvailable {
                    SearchingReader { isSearching in
                        let showsRecents = isSearching && term.isEmpty && !store.recentSearches.isEmpty
                        list(showsRecents: showsRecents)
                            .safeAreaInset(edge: .top, spacing: 0) {
                                if !showsRecents { chips }
                            }
                    }
                } else {
                    ContentUnavailableView(
                        "No App Store in This Build",
                        systemImage: "bag",
                        description: Text("This build of WarpSign was made without ipatool.")
                    )
                }
            }
            .navigationTitle("App Store")
            .searchable(text: $term, placement: .navigationBarDrawer(displayMode: .always), prompt: "Apps, Links, IDs, Bundle IDs")
            .onSubmit(of: .search) { search(term) }
            .onChange(of: term) { _, term in
                if term.isEmpty { clearSearch() }
            }
            .onChange(of: store.country) {
                if !searchedTerm.isEmpty { search(searchedTerm) }
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    accountButton
                }
            }
            .sheet(isPresented: $signingIn) { StoreSignInView() }
            .sheet(isPresented: $showingAccount) { StoreAccountView() }
            .task {
                if store.account == .unknown {
                    await store.refreshAccount()
                }
                // Starting ipatool (refreshAccount) kept the last crash's report
                lastCrash = AppStoreEngine.lastCrash
            }
            .onAppear { lastCrash = AppStoreEngine.lastCrash }
        }
    }

    private func list(showsRecents: Bool) -> some View {
        List {
            if showsRecents {
                recentSearches
            } else {
                content
            }
        }
        .listStyle(.insetGrouped)
        .listSectionSpacing(16)
        .contentMargins(.top, 4, for: .scrollContent)
        // UIKit's margins: 20 points on the widest iPhones, 16 on the rest
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in
            margin = width > 400 ? 20 : 16
        }
        .refreshable {
            if searchedTerm.isEmpty { await loadShelf(refresh: true) }
        }
        .task(id: "\(store.country)/\(shelf)/\(store.signedInAccount?.email ?? "")") {
            if searchedTerm.isEmpty { await loadShelf(refresh: false) }
        }
    }

    @ViewBuilder
    private var content: some View {
        if let crash = lastCrash {
            Section {
                NavigationLink {
                    StoreCrashView(report: crash)
                } label: {
                    Label("The App Store stopped WarpSign last time", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
        }
        if !store.downloads.isEmpty {
            Section("Downloading") {
                ForEach(store.downloads.values.sorted { $0.app.name < $1.app.name }, id: \.app.id) { download in
                    NavigationLink {
                        StoreAppView(app: download.app)
                    } label: {
                        StoreDownloadRow(download: download, showsApp: true)
                    }
                }
            }
        }

        if searchedTerm.isEmpty {
            shelfContent
        } else {
            searchContent
        }
    }

    // MARK: Chips

    private var chips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                countryChip
                if searchedTerm.isEmpty {
                    ForEach(StoreShelf.allCases) { item in
                        StoreChip(title: item.title, systemImage: item.systemImage, selected: shelf == item) {
                            shelf = item
                        }
                    }
                }
            }
        }
        .contentMargins(.horizontal, margin, for: .scrollContent)
        .padding(.top, 4)
        .padding(.bottom, 8)
        .background(Color(.systemGroupedBackground))
    }

    private var countryChip: some View {
        Menu {
            Picker("Store", selection: Binding(
                get: { store.chosenCountry },
                set: { store.chosenCountry = $0 }
            )) {
                Text("\(store.signedInAccount == nil ? "This iPhone's" : "Your Apple ID's") (\(StoreCountry.name(store.accountCountry)))")
                    .tag(String?.none)
                Divider()
                ForEach(StoreCountry.all, id: \.self) { code in
                    Text("\(StoreCountry.flag(code)) \(StoreCountry.name(code))").tag(Optional(code))
                }
            }
        } label: {
            StoreChipLabel(selected: false) {
                Text(StoreCountry.flag(store.country))
                Text(StoreCountry.name(store.country))
                Image(systemName: "chevron.down")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.secondary)
            }
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var accountButton: some View {
        switch store.account {
        case .signedIn(let info):
            Button {
                showingAccount = true
            } label: {
                Text(String((info.name.isEmpty ? info.email : info.name).prefix(1)).uppercased())
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.white)
                    .frame(width: 30, height: 30)
                    .background(Color.gray, in: Circle())
            }
            .accessibilityLabel("App Store Account")
        case .signedOut:
            Button("Sign In") { signingIn = true }
        case .unknown:
            EmptyView()
        }
    }

    // MARK: Charts and purchases

    @ViewBuilder
    private var shelfContent: some View {
        switch shelf {
        case .free, .paid:
            let chart = shelf == .free ? AppStoreCatalog.Chart.free : .paid
            if let apps = store.chart(chart) {
                Section {
                    ForEach(Array(apps.enumerated()), id: \.element.id) { index, app in
                        appLink(app, rank: index + 1)
                    }
                } header: {
                    countHeader("\(chart.title) in \(StoreCountry.name(store.country))")
                }
            } else {
                loadingOrError
            }
        case .purchased:
            purchasedContent
        }
    }

    @ViewBuilder
    private var purchasedContent: some View {
        if store.signedInAccount == nil {
            Section {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Sign in with the Apple ID you use in the App Store to see the apps it has got, and get them into your library.")
                        .foregroundStyle(.secondary)
                    Button("Sign In") { signingIn = true }
                        .buttonStyle(.borderedProminent)
                }
                .padding(.vertical, 6)
            }
        } else if let owned = store.owned {
            Section {
                if owned.isEmpty {
                    Text("Apps your Apple ID gets show up here.").foregroundStyle(.secondary)
                }
                ForEach(owned) { app in
                    appLink(app, country: store.accountCountry)
                }
            } header: {
                countHeader(store.ownedTotal > owned.count
                            ? "\(owned.count) most recent of \(store.ownedTotal)"
                            : "\(owned.count) apps")
            }
        } else {
            loadingOrError
        }
    }

    @ViewBuilder
    private var loadingOrError: some View {
        Section {
            if let shelfError {
                VStack(alignment: .leading, spacing: 8) {
                    Label(shelfError, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.secondary)
                    Button("Try Again") { Task { await loadShelf(refresh: true) } }
                }
                .padding(.vertical, 4)
            } else {
                HStack(spacing: 10) {
                    ProgressView()
                    Text(shelf == .purchased ? (store.readying ?? "Getting your apps") : "Loading")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
            }
        }
    }

    private func loadShelf(refresh: Bool) async {
        shelfError = nil
        do {
            switch shelf {
            case .free: try await store.loadChart(.free, refresh: refresh)
            case .paid: try await store.loadChart(.paid, refresh: refresh)
            case .purchased:
                guard store.signedInAccount != nil, refresh || store.owned == nil else { return }
                try await store.loadOwned(identity: account.identities[.development]?.data)
            }
        } catch {
            shelfError = error.userMessage
        }
    }

    // MARK: Search

    private var recentSearches: some View {
        Section {
            ForEach(store.recentSearches, id: \.self) { recent in
                Button {
                    term = recent
                    search(recent)
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "clock")
                            .foregroundStyle(.secondary)
                        Text(recent)
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Spacer()
                        Image(systemName: "arrow.up.left")
                            .font(.footnote)
                            .foregroundStyle(.tertiary)
                    }
                }
            }
            .onDelete { store.forgetSearches(at: $0) }
        } header: {
            HStack {
                countHeader("Recent Searches")
                Spacer()
                Button("Clear") { store.clearRecentSearches() }
                    .font(.subheadline)
                    .textCase(nil)
            }
        }
    }

    @ViewBuilder
    private var searchContent: some View {
        if searching && results.isEmpty {
            Section {
                ProgressView().frame(maxWidth: .infinity).padding(.vertical, 8)
            }
        } else if let searchError {
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Label(searchError, systemImage: "wifi.exclamationmark").foregroundStyle(.secondary)
                    Button("Try Again") { search(searchedTerm) }
                }
                .padding(.vertical, 4)
            }
        } else if results.isEmpty {
            Section {
                Text("Nothing for “\(searchedTerm)” in the \(StoreCountry.name(store.country)) App Store. You can also search with an App Store link, an App Store ID or a bundle ID.")
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 4)
            }
        } else {
            Section {
                ForEach(results) { app in
                    appLink(app, country: foundIn)
                }
            } header: {
                countHeader(results.count == 1 ? "1 result" : "\(results.count) results")
            } footer: {
                if let foundIn {
                    Text("Not in the \(StoreCountry.name(store.country)) App Store. Found in the \(StoreCountry.name(foundIn)) one.")
                }
            }
        }
    }

    private func search(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        searchedTerm = text
        searching = true
        searchError = nil
        results = []
        store.remember(search: text)
        Task {
            do {
                let found = try await store.find(text)
                // A newer search may have started meanwhile
                guard searchedTerm == text else { return }
                results = found.apps
                foundIn = found.foundIn
            } catch {
                guard searchedTerm == text else { return }
                foundIn = nil
                searchError = error.localizedDescription
            }
            searching = false
        }
    }

    private func clearSearch() {
        searchedTerm = ""
        results = []
        foundIn = nil
        searchError = nil
    }

    // MARK: Pieces

    private func appLink(_ app: CatalogApp, rank: Int? = nil, country: String? = nil) -> some View {
        NavigationLink {
            StoreAppView(app: app, country: country)
        } label: {
            StoreAppRow(app: app, rank: rank)
        }
    }

    private func countHeader(_ text: String) -> some View {
        Text(text)
            .font(.subheadline.weight(.medium))
            .textCase(nil)
    }
}

/// Whether the search field is in use, which only views inside
/// `.searchable` can tell
struct SearchingReader<Content: View>: View {
    @Environment(\.isSearching) private var isSearching
    @ViewBuilder let content: (Bool) -> Content

    var body: some View {
        content(isSearching)
    }
}

/// Go's report of the last crash in ipatool or Unicorn, to share
struct StoreCrashView: View {
    @Environment(\.dismiss) private var dismiss
    let report: String

    var body: some View {
        ScrollView {
            Text(report)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
        }
        .navigationTitle("Last Crash")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                ShareLink(item: report)
            }
            ToolbarItem(placement: .bottomBar) {
                Button("Forget It", role: .destructive) {
                    AppStoreEngine.forgetLastCrash()
                    dismiss()
                }
            }
        }
    }
}

/// What the tab shows when there's no search
enum StoreShelf: String, CaseIterable, Identifiable {
    case free, paid, purchased

    var id: String { rawValue }

    var title: String {
        switch self {
        case .free: "Top Free"
        case .paid: "Top Paid"
        case .purchased: "Purchased"
        }
    }

    var systemImage: String {
        switch self {
        case .free: "chart.line.uptrend.xyaxis"
        case .paid: "dollarsign.circle"
        case .purchased: "bag"
        }
    }
}

/// A capsule to choose with, filled when chosen
struct StoreChip: View {
    let title: String
    var systemImage: String? = nil
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            StoreChipLabel(selected: selected) {
                if let systemImage {
                    Image(systemName: systemImage).font(.footnote.weight(.semibold))
                }
                Text(title)
            }
        }
        .buttonStyle(.plain)
    }
}

struct StoreChipLabel<Content: View>: View {
    let selected: Bool
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: 5) {
            content
        }
        .font(.subheadline.weight(.medium))
        .foregroundStyle(selected ? Color(.systemBackground) : .primary)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(
            selected ? AnyShapeStyle(Color.primary) : AnyShapeStyle(Color(.secondarySystemGroupedBackground)),
            in: Capsule()
        )
        .contentShape(Capsule())
    }
}

/// The signed-in account, and the emulator getting apps needs
struct StoreAccountView: View {
    @Environment(\.dismiss) private var dismiss

    private var store: AppStoreModel { .shared }

    var body: some View {
        NavigationStack {
            Form {
                if let info = store.signedInAccount {
                    Section {
                        HStack(spacing: 14) {
                            Text(String((info.name.isEmpty ? info.email : info.name).prefix(1)).uppercased())
                                .font(.title2.weight(.semibold))
                                .foregroundStyle(.white)
                                .frame(width: 52, height: 52)
                                .background(Color.gray, in: Circle())
                            VStack(alignment: .leading, spacing: 2) {
                                Text(info.name.isEmpty ? info.email : info.name).font(.headline)
                                if !info.name.isEmpty {
                                    Text(info.email).font(.subheadline).foregroundStyle(.secondary)
                                }
                                Text("\(StoreCountry.flag(store.accountCountry)) \(StoreCountry.name(store.accountCountry)) App Store")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 4)
                    } footer: {
                        Text("Apps are got from this account's store, whichever store you browse.")
                    }
                    Section {
                        Button("Sign Out", role: .destructive) {
                            Task {
                                await store.signOut()
                                dismiss()
                            }
                        }
                    }
                }
                Section {
                    LabeledContent("Emulator", value: emulatorStatus)
                    if let readying = store.readying {
                        HStack(spacing: 8) {
                            ProgressView()
                            Text(readying).foregroundStyle(.secondary)
                        }
                    }
                } header: {
                    Text("Getting Apps")
                } footer: {
                    Text("Signing in and getting apps run Apple's own code in Unicorn, an emulator WarpSign downloads once (about 2 MB) and signs with your development certificate. Browsing needs none of this.")
                }
            }
            .navigationTitle("Account")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private var emulatorStatus: String {
        if store.isReady { return "Ready" }
        if UnicornRuntime.isReady { return "Loaded" }
        return UnicornRuntime.isDownloaded ? "Sets up when needed" : "Downloads when needed"
    }
}

/// Country names and flags for stores
enum StoreCountry {
    static func name(_ code: String) -> String {
        Locale.current.localizedString(forRegionCode: code) ?? code
    }

    /// The flag emoji, from the code's letters
    static func flag(_ code: String) -> String {
        String(String.UnicodeScalarView(code.uppercased().unicodeScalars.compactMap {
            UnicodeScalar(0x1F1E6 - 0x41 + $0.value)
        }))
    }

    /// Every store, by name
    static let all: [String] = AppStoreCatalog.allCountries.sorted {
        name($0).localizedStandardCompare(name($1)) == .orderedAscending
    }
}

/// An app in a list: icon, name, category, version and size, and Get
struct StoreAppRow: View {
    let app: CatalogApp
    /// Its place in a chart
    var rank: Int? = nil

    private var details: String {
        var parts: [String] = []
        if let rating = app.averageUserRating, let count = app.userRatingCount, count > 0 {
            parts.append("★ \(rating.formatted(.number.precision(.fractionLength(1))))")
        }
        if let version = app.version { parts.append("v\(version)") }
        if let size = app.size { parts.append(ByteCountFormatter.file(size)) }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        HStack(spacing: 12) {
            if let rank {
                Text("\(rank)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 22, alignment: .center)
            }
            RemoteIconView(url: app.iconURL, size: 56)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.name)
                    .font(.body.weight(.semibold))
                    .lineLimit(2)
                if let subtitle = app.primaryGenreName ?? app.developer {
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if !details.isEmpty {
                    Text(details)
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            StoreGetButton(app: app)
        }
        .padding(.vertical, 4)
    }
}

/// A download's progress, or why it failed
struct StoreDownloadRow: View {
    let download: StoreDownload
    /// With the app's icon and name, for lists of downloads
    var showsApp = false

    private var store: AppStoreModel { .shared }

    var body: some View {
        HStack(spacing: 12) {
            if showsApp {
                RemoteIconView(url: download.app.iconURL, size: 40)
            }
            VStack(alignment: .leading, spacing: 6) {
                if showsApp {
                    Text(download.app.name).font(.headline)
                }
                if case .failed = download.phase {
                    Label(download.status(readying: nil), systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline)
                        .foregroundStyle(.orange)
                } else {
                    if let fraction = download.fraction {
                        ProgressView(value: fraction)
                    } else {
                        ProgressView(value: nil as Double?)
                            .progressViewStyle(.linear)
                    }
                    Text(download.status(readying: store.readying))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if !download.isActive {
                Button {
                    store.dismissDownload(download.app)
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.borderless)
            }
        }
    }
}

/// Signing in to the App Store, with the two-factor code when Apple asks
struct StoreSignInView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AccountStore.self) private var account
    @State private var email = ""
    @State private var password = ""
    @State private var code = ""
    @State private var needsCode = false
    @State private var running = false
    @State private var error: String?

    private var store: AppStoreModel { .shared }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if needsCode {
                        TextField("Code", text: $code)
                            .keyboardType(.numberPad)
                            .textContentType(.oneTimeCode)
                    } else {
                        TextField("Apple ID", text: $email)
                            .textContentType(.username)
                            .keyboardType(.emailAddress)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        SecureField("Password", text: $password)
                            .textContentType(.password)
                    }
                } footer: {
                    if needsCode {
                        Text("Enter the code Apple sent to your devices.")
                    } else {
                        Text("The Apple ID you use in the App Store. It goes only to Apple; ipatool keeps the session on this iPhone, encrypted.")
                    }
                }

                Section {
                    Button {
                        signIn()
                    } label: {
                        HStack {
                            Text(needsCode ? "Continue" : "Sign In")
                            Spacer()
                            if running { ProgressView() }
                        }
                    }
                    .disabled(running || email.isEmpty || password.isEmpty || (needsCode && code.isEmpty))
                    if running, let readying = store.readying {
                        Text(readying).font(.footnote).foregroundStyle(.secondary)
                    }
                    if let error {
                        Text(error).font(.footnote).foregroundStyle(.red)
                    }
                } footer: {
                    Text("Signing in runs Apple's code in Unicorn, an emulator WarpSign downloads once (about 2 MB) and signs with your development certificate.")
                }
            }
            .navigationTitle("App Store Account")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .interactiveDismissDisabled(running)
        }
    }

    private func signIn() {
        running = true
        error = nil
        let identity = account.identities[.development]?.data
        Task {
            do {
                try await store.signIn(
                    email: email, password: password, code: needsCode ? code : nil, identity: identity)
                dismiss()
            } catch WarpsignError.StoreCodeRequired {
                needsCode = true
            } catch {
                self.error = error.userMessage
            }
            running = false
        }
    }
}

