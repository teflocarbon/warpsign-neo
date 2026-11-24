import SwiftUI

/// The App Store tab: search the store, see your account and the apps it
/// has got, and get apps into the library to sign. Browsing needs no Apple
/// ID; getting apps does, through ipatool.
struct AppStoreView: View {
    @Environment(AccountStore.self) private var account
    @State private var term = ""
    @State private var searchedTerm = ""
    @State private var results: [CatalogApp] = []
    @State private var searching = false
    @State private var searchError: String?
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
            .searchable(text: $term, prompt: "Apps")
            .onSubmit(of: .search, search)
            .onChange(of: term) { _, term in
                if term.isEmpty {
                    searchedTerm = ""
                    results = []
                    searchError = nil
                }
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
                        Text("\(countryName) App Store").font(.caption).foregroundStyle(.secondary)
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

    private var countryName: String {
        Locale.current.localizedString(forRegionCode: store.country) ?? store.country
    }

    // MARK: Search

    @ViewBuilder
    private var searchResults: some View {
        if searching && results.isEmpty {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let searchError {
            ContentUnavailableView("Couldn't Search", systemImage: "wifi.exclamationmark", description: Text(searchError))
        } else if results.isEmpty {
            ContentUnavailableView.search(text: searchedTerm)
        } else {
            List(results) { app in
                NavigationLink {
                    StoreAppView(app: app)
                } label: {
                    StoreAppRow(app: app)
                }
            }
            .listStyle(.plain)
        }
    }

    private func search() {
        let term = term.trimmingCharacters(in: .whitespaces)
        guard !term.isEmpty else { return }
        searchedTerm = term
        searching = true
        searchError = nil
        Task {
            do {
                results = try await AppStoreCatalog.search(term, country: store.country)
            } catch {
                results = []
                searchError = error.localizedDescription
            }
            searching = false
        }
    }
}

/// An app in a list: icon, name, developer, rating, and Get
struct StoreAppRow: View {
    let app: CatalogApp

    var body: some View {
        HStack(spacing: 12) {
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
                                    StoreAppView(app: app)
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
