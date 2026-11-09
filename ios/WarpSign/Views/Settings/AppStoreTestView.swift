import SwiftUI

/// Tries ipatool on the device, a step at a time, before the App Store
/// gets its own screen. Preparing needs no Apple ID.
struct AppStoreTestView: View {
    @Environment(AccountStore.self) private var account
    @State private var running = false
    @State private var unicornStep: String?
    @State private var results: [Result] = []
    /// ipatool's cache, while Prepare fills it
    @State private var cached: Int64?
    /// ipatool's cache when the screen opened or after Prepare
    @State private var cacheOnDisk: Int64?

    private var unicornStatus: String {
        if UnicornRuntime.isReady { return "Ready" }
        if emulatorState().loaded { return "Loaded, needs JIT" }
        // Loading doesn't outlast the app, so each launch sets it up again
        return UnicornRuntime.isDownloaded ? "Downloaded, not set up yet" : "Not downloaded"
    }

    @State private var email = ""
    @State private var password = ""
    @State private var code = ""
    @State private var needsCode = false
    @State private var term = ""
    @State private var apps: [StoreAppInfo] = []
    @State private var purchase = false

    struct Result: Identifiable {
        let id = UUID()
        let step: String
        let ok: Bool
        let detail: String
    }

    var body: some View {
        List {
            Section {
                LabeledContent("ipatool in this build", value: AppStoreEngine.isAvailable ? "Yes" : "No")
                LabeledContent("Runtime and Apple assets") {
                    if let cacheOnDisk {
                        Text(cacheOnDisk == 0 ? "Not downloaded" : "Downloaded, \(cacheOnDisk.formatted(.byteCount(style: .file)))")
                    } else {
                        ProgressView()
                    }
                }
                Button {
                    prepare()
                } label: {
                    Label("Prepare", systemImage: "shippingbox")
                }
                .disabled(running || !AppStoreEngine.isAvailable)
                Button {
                    run("Account") { client in
                        try client.account().map { "Signed in as \($0.email)" } ?? "Not signed in"
                    }
                } label: {
                    Label("Check Account", systemImage: "person.crop.circle")
                }
                .disabled(running || !AppStoreEngine.isAvailable)
            } footer: {
                Text("Prepare loads ipatool's runtime and fetches the Apple assets it needs (about 190 MB the first time). It doesn't contact your Apple ID.")
            }

            Section {
                LabeledContent("Unicorn", value: unicornStatus)
                Button {
                    setUpUnicorn()
                } label: {
                    Label("Set Up Unicorn", systemImage: "cpu")
                }
                .disabled(running || UnicornRuntime.isReady)
                if let unicornStep {
                    Text(unicornStep).font(.footnote).foregroundStyle(.secondary)
                }
            } footer: {
                Text("Signing in and downloading run Apple's code in Unicorn, which WarpSign downloads (about 2 MB), signs with your development certificate and loads. It needs JIT: StikDebug opens to enable it.")
            }

            Section {
                if needsCode {
                    TextField("Two-factor code", text: $code)
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
                Button {
                    signIn()
                } label: {
                    Label(needsCode ? "Sign In with Code" : "Sign In", systemImage: "person.badge.key")
                }
                .disabled(running || !UnicornRuntime.isReady || email.isEmpty || password.isEmpty
                    || (needsCode && code.isEmpty))
                Button(role: .destructive) {
                    run("Sign Out") { client in
                        try client.logout()
                        return "Signed out"
                    }
                } label: {
                    Label("Sign Out", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .disabled(running)
            } header: {
                Text("App Store Account")
            } footer: {
                Text("Your Apple ID for the App Store, kept by ipatool in this app, encrypted. Needs Unicorn set up. Apple may ask for a two-factor code.")
            }

            Section {
                TextField("Search the App Store", text: $term)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit(search)
                Toggle("Get free apps I don't own", isOn: $purchase)
                ForEach(apps, id: \.id) { app in
                    Button {
                        download(app)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(app.name)
                            Text("\(app.bundleId) · \(app.version) · \(app.price == 0 ? "Free" : String(format: "%.2f", app.price))")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .disabled(running || !UnicornRuntime.isReady)
                }
            } header: {
                Text("Search and Download")
            } footer: {
                Text("Tap an app to download it into WarpSign's Documents folder (App Store). Getting a free app adds it to your Apple ID's purchases, as the App Store does.")
            }

            if running {
                Section {
                    ProgressView("Running…")
                    if let cached {
                        LabeledContent("ipatool's cache", value: cached.formatted(.byteCount(style: .file)))
                    }
                }
            }

            if !results.isEmpty {
                Section("Results") {
                    ForEach(results.reversed()) { result in
                        VStack(alignment: .leading, spacing: 4) {
                            Label(result.step, systemImage: result.ok ? "checkmark.circle" : "xmark.octagon")
                                .foregroundStyle(result.ok ? .green : .red)
                            Text(result.detail)
                                .font(.footnote.monospaced())
                                .textSelection(.enabled)
                        }
                    }
                }
            }
        }
        .navigationTitle("App Store (ipatool)")
        .task {
            cacheOnDisk = await Task.detached { Self.cacheSize() }.value
        }
    }

    private func setUpUnicorn() {
        guard let identity = account.identities[.development]?.data else {
            results.append(Result(step: "Unicorn", ok: false, detail: UnicornRuntime.Failure.noCertificate.localizedDescription))
            return
        }
        running = true
        Task {
            let started = Date()
            var outcome: (Bool, String)
            do {
                try await UnicornRuntime.prepare(identity: identity) { unicornStep = $0 }
                outcome = (true, "Loaded, with JIT")
            } catch {
                outcome = (false, error.userMessage)
            }
            let seconds = String(format: "%.1f s", Date().timeIntervalSince(started))
            results.append(Result(step: "Unicorn", ok: outcome.0, detail: "\(outcome.1) (\(seconds))"))
            unicornStep = nil
            running = false
        }
    }

    /// Prepare, showing ipatool's cache growing (it reports no progress)
    private func prepare() {
        let watcher = Task {
            while !Task.isCancelled {
                cached = await Task.detached { Self.cacheSize() }.value
                try? await Task.sleep(for: .seconds(1))
            }
        }
        run("Prepare", then: {
            watcher.cancel()
            cacheOnDisk = cached
            cached = nil
        }) { client in
            try client.prepare()
            return "Runtime loaded, Apple's assets cached"
        }
    }

    /// The size of ipatool's cache (os.UserCacheDir()/ipatool)
    nonisolated private static func cacheSize() -> Int64 {
        let folder = URL.cachesDirectory.appendingPathComponent("ipatool", isDirectory: true)
        guard let files = FileManager.default.enumerator(
            at: folder, includingPropertiesForKeys: [.fileSizeKey]) else { return 0 }
        var total: Int64 = 0
        for case let file as URL in files {
            total += Int64((try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
        }
        return total
    }

    private func signIn() {
        let email = email, password = password
        let code = needsCode ? code : nil
        running = true
        Task {
            let started = Date()
            let outcome = await Task.detached { () -> (Bool, String, Bool) in
                do {
                    let account = try AppStoreEngine.client().login(email: email, password: password, code: code)
                    return (true, "Signed in as \(account.name)", false)
                } catch WarpsignError.StoreCodeRequired {
                    return (false, "Apple sent a two-factor code: enter it", true)
                } catch {
                    return (false, error.userMessage, false)
                }
            }.value
            let seconds = String(format: "%.1f s", Date().timeIntervalSince(started))
            results.append(Result(step: "Sign In", ok: outcome.0, detail: "\(outcome.1) (\(seconds))"))
            needsCode = outcome.2
            if outcome.0 {
                self.password = ""
                self.code = ""
            }
            running = false
        }
    }

    private func search() {
        let term = term.trimmingCharacters(in: .whitespaces)
        guard !term.isEmpty else { return }
        running = true
        Task {
            let outcome = await Task.detached { () -> Swift.Result<[StoreAppInfo], Error> in
                Swift.Result { try AppStoreEngine.client().search(term: term, limit: 10) }
            }.value
            switch outcome {
            case .success(let found):
                apps = found
                results.append(Result(step: "Search", ok: true, detail: "\(found.count) apps for “\(term)”"))
            case .failure(let error):
                results.append(Result(step: "Search", ok: false, detail: error.userMessage))
            }
            running = false
        }
    }

    private func download(_ app: StoreAppInfo) {
        let purchase = purchase
        let folder = URL.documentsDirectory.appendingPathComponent("App Store", isDirectory: true)
        run("Download \(app.name)") { client in
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let download = try client.download(
                app: String(app.id), version: nil, directory: folder.path, purchase: purchase)
            let size = (try? FileManager.default.attributesOfItem(atPath: download.path)[.size] as? Int64) ?? 0
            return "\(URL(fileURLWithPath: download.path).lastPathComponent), "
                + size.formatted(.byteCount(style: .file))
                + (download.purchased ? ", got it first" : "")
        }
    }

    private func run(
        _ step: String,
        then finish: @escaping @MainActor () -> Void = {},
        _ work: @escaping @Sendable (AppStoreClient) throws -> String
    ) {
        running = true
        Task {
            let started = Date()
            let outcome = await Task.detached { () -> (Bool, String) in
                do {
                    return (true, try work(try AppStoreEngine.client()))
                } catch {
                    return (false, error.userMessage)
                }
            }.value
            let seconds = String(format: "%.1f s", Date().timeIntervalSince(started))
            results.append(Result(step: step, ok: outcome.0, detail: "\(outcome.1) (\(seconds))"))
            finish()
            running = false
        }
    }
}
