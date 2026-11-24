import SwiftUI

/// The App Store tab: search the store and its charts, see your account and
/// the apps it has got, and get apps into the library to sign. Browsing
/// needs no Apple ID; getting apps does, through ipatool.
struct AppStoreView: View {
    @Environment(AccountStore.self) private var account
    @State private var term = ""
    @State private var searchedTerm = ""
    @State private var results: [CatalogApp] = []
    /// The store an app named by link or ID was found in, when not this one
    @State private var foundIn: String?
    @State private var searching = false
    @State private var searchError: String?
    @State private var chartErrors: [AppStoreCatalog.Chart: String] = [:]
    @State private var signingIn = false

    private var store: AppStoreModel { .shared }

    var body: some View {
        NavigationStack {
            Group {
                if !AppStoreEngine.isAvailable {
                    ContentUnavailableView(
                        "No App Store in This Build",
                        systemImage: "bag",
                        description: Text("This build of WarpSign was made without ipatool.")
                    )
                } else if !searchedTerm.isEmpty {
                    searchResults
                } else {
                    home
                }
            }
            .navigationTitle("App Store")
            .searchable(text: $term, prompt: "Apps, Links, IDs, Bundle IDs")
            .searchSuggestions { recentSearches }
            .onSubmit(of: .search) { search(term) }
            .onChange(of: term) { _, term in
                if term.isEmpty {
                    searchedTerm = ""
                    results = []
                    foundIn = nil
                    searchError = nil
                }
            }
            .onChange(of: store.country) {
                if !searchedTerm.isEmpty { search(searchedTerm) }
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    accountButton
                }
            }
            .sheet(isPresented: $signingIn) {
                StoreSignInView()
            }
            .task {
                if store.account == .unknown {
                    await store.refreshAccount()
                }
            }
        }
    }

    @ViewBuilder
    private var accountButton: some View {
        switch store.account {
        case .signedIn(let info):
            Menu {
                Section(info.email) {
                    Button(role: .destructive) {
                        Task { await store.signOut() }
                    } label: {
                        Label("Sign Out", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            } label: {
                Image(systemName: "person.crop.circle.fill")
            }
        case .signedOut:
            Button {
                signingIn = true
            } label: {
                Image(systemName: "person.crop.circle")
            }
        case .unknown:
            EmptyView()
        }
    }

    // MARK: Home

    private var home: some View {
        List {
            accountSection
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
            if store.signedInAccount != nil {
                Section {
                    NavigationLink {
                        OwnedAppsView()
                    } label: {
                        Label("Purchased", systemImage: "square.stack")
                    }
                } footer: {
                    Text("The apps your Apple ID has got, free or paid.")
                }
            }
            ForEach(AppStoreCatalog.Chart.allCases) { chart in
                chartSection(chart)
            }
            StoreCountrySection()
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
                Text("Signing in and getting apps run Apple's own code in Unicorn, an emulator WarpSign downloads once (about 2 MB) and signs with your development certificate. Searching needs none of this.")
            }
        }
        .refreshable { await loadCharts(refresh: true) }
        .task(id: store.country) { await loadCharts(refresh: false) }
    }

    /// The first few of a chart, and the rest a tap away
    @ViewBuilder
    private func chartSection(_ chart: AppStoreCatalog.Chart) -> some View {
        Section {
            if let apps = store.chart(chart) {
                ForEach(Array(apps.prefix(5).enumerated()), id: \.element.id) { index, app in
                    NavigationLink {
                        StoreAppView(app: app)
                    } label: {
                        StoreAppRow(app: app, rank: index + 1)
                    }
                }
                if apps.count > 5 {
                    NavigationLink("See All") {
                        StoreChartView(chart: chart)
                    }
                }
            } else if let error = chartErrors[chart] {
                Label(error, systemImage: "wifi.exclamationmark")
                    .foregroundStyle(.secondary)
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        } header: {
            Text("\(chart.title) in \(countryName)")
        }
    }

    private func loadCharts(refresh: Bool) async {
        for chart in AppStoreCatalog.Chart.allCases {
            do {
                try await store.loadChart(chart, refresh: refresh)
                chartErrors[chart] = nil
            } catch {
                chartErrors[chart] = error.localizedDescription
            }
        }
    }

    private var emulatorStatus: String {
        if store.isReady { return "Ready" }
        if UnicornRuntime.isReady { return "Ready, App Store not loaded yet" }
        return UnicornRuntime.isDownloaded ? "Sets up when needed" : "Downloads when needed (2 MB)"
    }

    @ViewBuilder
    private var accountSection: some View {
        Section {
            switch store.account {
            case .unknown:
                ProgressView()
            case .signedIn(let info):
                HStack(spacing: 12) {
                    Image(systemName: "person.crop.circle.fill")
                        .font(.system(size: 40))
                        .foregroundStyle(.tint)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(info.name.isEmpty ? info.email : info.name).font(.headline)
                        Text(info.email).font(.subheadline).foregroundStyle(.secondary)
                        Text("\(StoreCountry.name(store.accountCountry)) App Store")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 4)
            case .signedOut:
                VStack(alignment: .leading, spacing: 10) {
                    Text("Not Signed In").font(.headline)
                    Text("Sign in with the Apple ID you use in the App Store to get apps into your library. You can search and look at apps without signing in.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    Button("Sign In") { signingIn = true }
                        .buttonStyle(.borderedProminent)
                }
                .padding(.vertical, 4)
            }
        }
    }

    private var countryName: String { StoreCountry.name(store.country) }

    // MARK: Search

    @ViewBuilder
    private var recentSearches: some View {
        if term.isEmpty && !store.recentSearches.isEmpty {
            Section("Recent") {
                ForEach(store.recentSearches, id: \.self) { recent in
                    Button {
                        term = recent
                        search(recent)
                    } label: {
                        Label(recent, systemImage: "clock.arrow.circlepath")
                    }
                }
                Button("Clear Recent Searches", role: .destructive) {
                    store.clearRecentSearches()
                }
            }
        }
    }

    @ViewBuilder
    private var searchResults: some View {
        if searching && results.isEmpty {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let searchError {
            ContentUnavailableView {
                Label("Couldn't Search", systemImage: "wifi.exclamationmark")
            } description: {
                Text(searchError)
            } actions: {
                Button("Try Again") { search(searchedTerm) }
            }
        } else if results.isEmpty {
            ContentUnavailableView {
                Label("No Results", systemImage: "magnifyingglass")
            } description: {
                Text("Nothing for “\(searchedTerm)” in the \(countryName) App Store. You can also search with an App Store link, an App Store ID or a bundle ID.")
            }
        } else {
            List {
                Section {
                    ForEach(results) { app in
                        NavigationLink {
                            StoreAppView(app: app, country: foundIn)
                        } label: {
                            StoreAppRow(app: app)
                        }
                    }
                } header: {
                    if let foundIn {
                        Text("Not in the \(countryName) App Store. Found in the \(StoreCountry.name(foundIn)) one.")
                            .textCase(nil)
                    }
                }
            }
            .listStyle(.plain)
        }
    }

    private func search(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        searchedTerm = text
        searching = true
        searchError = nil
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
                results = []
                foundIn = nil
                searchError = error.localizedDescription
            }
            searching = false
        }
    }
}

/// Country names for stores
enum StoreCountry {
    static func name(_ code: String) -> String {
        Locale.current.localizedString(forRegionCode: code) ?? code
    }

    /// Every store, by name
    static let all: [String] = AppStoreCatalog.allCountries.sorted {
        name($0).localizedStandardCompare(name($1)) == .orderedAscending
    }
}

/// Which country's App Store to browse
struct StoreCountrySection: View {
    private var store: AppStoreModel { .shared }

    var body: some View {
        Section {
            Picker("Store", selection: Binding(
                get: { store.chosenCountry },
                set: { store.chosenCountry = $0 }
            )) {
                Text(store.signedInAccount == nil
                     ? "This iPhone's (\(StoreCountry.name(store.accountCountry)))"
                     : "Your Apple ID's (\(StoreCountry.name(store.accountCountry)))")
                    .tag(String?.none)
                ForEach(StoreCountry.all, id: \.self) { code in
                    Text(StoreCountry.name(code)).tag(Optional(code))
                }
            }
            .pickerStyle(.navigationLink)
        } footer: {
            Text("Search and the charts show this country's App Store. Getting an app always uses your Apple ID's own store, which may not have it.")
        }
    }
}

/// A whole chart, up to 100 apps
struct StoreChartView: View {
    let chart: AppStoreCatalog.Chart
    @State private var error: String?

    private var store: AppStoreModel { .shared }

    var body: some View {
        Group {
            if let apps = store.chart(chart) {
                List(Array(apps.enumerated()), id: \.element.id) { index, app in
                    NavigationLink {
                        StoreAppView(app: app)
                    } label: {
                        StoreAppRow(app: app, rank: index + 1)
                    }
                }
                .listStyle(.plain)
                .refreshable { await load(refresh: true) }
            } else if let error {
                ContentUnavailableView {
                    Label("Couldn't Get the Chart", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error)
                } actions: {
                    Button("Try Again") { Task { await load(refresh: true) } }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("\(chart.title) in \(StoreCountry.name(store.country))")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: store.country) { await load(refresh: false) }
    }

    private func load(refresh: Bool) async {
        error = nil
        do {
            try await store.loadChart(chart, refresh: refresh)
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// An app in a list: icon, name, developer, rating, and Get
struct StoreAppRow: View {
    let app: CatalogApp
    /// Its place in a chart
    var rank: Int? = nil

    var body: some View {
        HStack(spacing: 12) {
            if let rank {
                Text("\(rank)")
                    .font(.headline.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 24, alignment: .trailing)
            }
            RemoteIconView(url: app.iconURL, size: 60)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.name).font(.headline).lineLimit(1)
                if let developer = app.developer {
                    Text(developer).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                }
                if let rating = app.averageUserRating, let count = app.userRatingCount, count > 0 {
                    HStack(spacing: 4) {
                        StarRating(rating: rating)
                        Text(count.formatted(.number.notation(.compactName)))
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                } else if let genre = app.primaryGenreName {
                    Text(genre).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 8)
            StoreGetButton(app: app)
        }
        .padding(.vertical, 2)
    }
}

/// Five stars, filled to the rating
struct StarRating: View {
    let rating: Double

    var body: some View {
        HStack(spacing: 1) {
            ForEach(0..<5) { index in
                let fill = rating - Double(index)
                Image(systemName: fill >= 0.75 ? "star.fill" : fill >= 0.25 ? "star.leadinghalf.filled" : "star")
            }
        }
        .imageScale(.small)
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

/// The apps the account has got
struct OwnedAppsView: View {
    @Environment(AccountStore.self) private var account
    @State private var error: String?
    @State private var loading = false

    private var store: AppStoreModel { .shared }

    var body: some View {
        Group {
            if let owned = store.owned {
                if owned.isEmpty {
                    ContentUnavailableView("No Apps", systemImage: "square.stack", description: Text("Apps your Apple ID gets show up here."))
                } else {
                    List {
                        Section {
                            ForEach(owned) { app in
                                NavigationLink {
                                    StoreAppView(app: app, country: store.accountCountry)
                                } label: {
                                    StoreAppRow(app: app)
                                }
                            }
                        } footer: {
                            if store.ownedTotal > owned.count {
                                Text("The \(owned.count) most recent of \(store.ownedTotal).")
                            }
                        }
                    }
                    .listStyle(.plain)
                }
            } else if let error {
                ContentUnavailableView {
                    Label("Couldn't List Your Apps", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(error)
                } actions: {
                    Button("Try Again") { load() }
                }
            } else {
                VStack(spacing: 12) {
                    ProgressView()
                    Text(store.readying ?? "Getting your apps")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Purchased")
        .refreshable { await reload() }
        .onAppear {
            if store.owned == nil { load() }
        }
    }

    private func load() {
        guard !loading else { return }
        Task { await reload() }
    }

    private func reload() async {
        loading = true
        error = nil
        do {
            try await store.loadOwned(identity: account.identities[.development]?.data)
        } catch {
            self.error = error.userMessage
        }
        loading = false
    }
}
