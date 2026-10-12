import Foundation
import Observation

/// A signing identity kept in the Keychain
struct StoredIdentity: Codable, Sendable {
    var certificate: Data
    var privateKey: Data
    /// The user confirmed they saved the backup
    var backedUp: Bool = false

    var data: IdentityData {
        IdentityData(certificate: certificate, privateKey: privateKey)
    }
}

/// The Apple ID, its developer teams and the signing identities: everything
/// that needs the user's account
@MainActor
@Observable
final class AccountStore {
    enum Phase: Equatable {
        case signedOut
        /// Checking the saved session on launch
        case restoring
        case signedIn
    }

    private(set) var phase: Phase = .signedOut
    private(set) var appleId: String?
    private(set) var teams: [TeamInfo] = []
    var selectedTeamId: String? {
        didSet { defaults.set(selectedTeamId, forKey: Keys.team) }
    }
    /// Set when a saved session stopped working
    private(set) var sessionProblem: String?

    private(set) var identities: [CertificateType: StoredIdentity] = [:]
    /// Certificate serial → whether the portal still lists it
    private(set) var portalStatus: [String: Bool] = [:]
    /// Moves to a new certificate under way, by kind
    private(set) var migrations: [String: CertificateMigration] = [:]
    /// Developer option: offer migration as if each certificate were expiring
    var forceMigration: Bool {
        didSet { defaults.set(forceMigration, forKey: Keys.forceMigration) }
    }

    @ObservationIgnored private var account: AppleAccount?
    /// The password from the sign-in under way, kept if it's remembered
    @ObservationIgnored private var pendingPassword: String?
    /// Told after every sign-in: updates waiting on one carry on
    @ObservationIgnored var onSignIn: (@MainActor () -> Void)?
    /// A silent sign-in under way, which concurrent calls wait for
    @ObservationIgnored private var reauthentication: Task<Bool, Never>?
    /// Screenshots: certificate details without real certificates
    @ObservationIgnored private var demoInfos: [CertificateType: IdentityInfo] = [:]
    @ObservationIgnored private let transport = URLSessionTransport()
    @ObservationIgnored private let defaults = UserDefaults.standard

    private enum Keys {
        static let appleId = "account.appleId"
        static let team = "account.team"
        static let session = "session"
        static let password = "account.password"
        static let rememberPassword = "account.rememberPassword"
        static let silentSignInPaused = "account.silentSignInPaused"
        static func identity(_ kind: CertificateType) -> String {
            kind == .development ? "identity.development" : "identity.distribution"
        }
        static func ppq(_ team: String) -> String { "ppq.\(team)" }
        static let migrations = "certificateMigrations"
        static let forceMigration = "developer.forceMigration"
    }

    init() {
        appleId = defaults.string(forKey: Keys.appleId)
        selectedTeamId = defaults.string(forKey: Keys.team)
        forceMigration = UserDefaults.standard.bool(forKey: Keys.forceMigration)
        if let data = UserDefaults.standard.data(forKey: Keys.migrations),
           let saved = try? JSONDecoder().decode([String: CertificateMigration].self, from: data) {
            migrations = saved
        }
        for kind in [CertificateType.development, .distribution] {
            if let data = Keychain.data(for: Keys.identity(kind)),
               let identity = try? JSONDecoder().decode(StoredIdentity.self, from: data) {
                identities[kind] = identity
            }
        }
    }

    init(demoAppleId: String, team: TeamInfo, identities infos: [CertificateType: IdentityInfo]) {
        forceMigration = false
        phase = .signedIn
        appleId = demoAppleId
        teams = [team]
        selectedTeamId = team.teamId
        demoInfos = infos
        for kind in infos.keys {
            identities[kind] = StoredIdentity(certificate: Data(), privateKey: Data(), backedUp: true)
        }
        portalStatus = Dictionary(uniqueKeysWithValues: infos.values.map { ($0.serialNumber, true) })
    }

    var selectedTeam: TeamInfo? {
        teams.first { $0.teamId == selectedTeamId } ?? teams.first
    }

    var isSignedIn: Bool { phase == .signedIn && selectedTeam != nil }

    /// Whether to keep the password in the Keychain, to sign in again by
    /// itself when Apple ends the session
    var rememberPassword: Bool {
        defaults.object(forKey: Keys.rememberPassword) as? Bool ?? true
    }

    /// The remembered password, to fill in when signing in again by hand
    var savedPassword: String? {
        Keychain.data(for: Keys.password).flatMap { String(data: $0, encoding: .utf8) }
    }

    /// Set when signing in again by itself didn't work in a way that trying
    /// again wouldn't fix (Apple refused, or wants a code): it waits for
    /// the user to sign in, as repeated attempts can lock the Apple ID or
    /// send a code to their devices each time
    private var silentSignInPaused: Bool {
        get { defaults.bool(forKey: Keys.silentSignInPaused) }
        set { defaults.set(newValue, forKey: Keys.silentSignInPaused) }
    }

    /// Whether the team's profiles have PPQ checking, once a sign has told us
    func teamUsesPPQ(_ teamId: String) -> Bool? {
        defaults.object(forKey: Keys.ppq(teamId)) as? Bool
    }

    func recordPPQ(_ uses: Bool, team teamId: String) {
        defaults.set(uses, forKey: Keys.ppq(teamId))
    }

    // MARK: Signing in

    /// Resume the saved session, if there is one
    func restore() async {
        guard phase == .signedOut,
              let data = Keychain.data(for: Keys.session),
              let json = String(data: data, encoding: .utf8)
        else { return }
        phase = .restoring
        do {
            let account = try AppleAccount.restore(transport: transport, sessionJson: json)
            let teams = try await background { try account.checkSession() }
            signedIn(account, teams: teams)
        } catch {
            if Self.isExpired(error), await reauthenticate() { return }
            phase = .signedOut
            sessionProblem = sessionProblem ?? error.userMessage
        }
    }

    /// Start signing in. Returns the two-factor challenge if a code is needed.
    func signIn(appleId: String, password: String, remember: Bool) async throws -> TwoFactorInfo? {
        // The last session's two-factor trust, so Apple needn't ask for a code
        let account = AppleAccount.newTrusted(transport: transport, sessionJson: savedSessionJson())
        let id = appleId.trimmingCharacters(in: .whitespaces)
        defaults.set(remember, forKey: Keys.rememberPassword)
        silentSignInPaused = false
        pendingPassword = password
        let result = try await background { try account.signIn(account: id, password: password) }
        self.account = account
        self.appleId = id
        defaults.set(id, forKey: Keys.appleId)
        switch result {
        case .signedIn(let teams):
            signedIn(account, teams: teams)
            return nil
        case .needsCode(let challenge):
            return challenge
        }
    }

    func requestSms(_ phone: TrustedPhoneInfo) async throws {
        guard let account else { return }
        try await background { try account.requestSms(phoneId: phone.id) }
    }

    /// Finish signing in with a code from a trusted device, or by text
    /// message to `phone`
    func verify(code: String, phone: TrustedPhoneInfo?) async throws {
        guard let account else { return }
        let teams = try await background { () throws -> [TeamInfo] in
            if let phone {
                return try account.verifySmsCode(phoneId: phone.id, code: code)
            }
            return try account.verifyDeviceCode(code: code)
        }
        signedIn(account, teams: teams)
    }

    private func signedIn(_ account: AppleAccount, teams: [TeamInfo]) {
        self.account = account
        self.teams = teams
        if selectedTeamId == nil || !teams.contains(where: { $0.teamId == selectedTeamId }) {
            selectedTeamId = teams.first?.teamId
        }
        phase = .signedIn
        sessionProblem = nil
        saveSession()
        if let password = pendingPassword {
            pendingPassword = nil
            try? Keychain.set(rememberPassword ? Data(password.utf8) : nil, for: Keys.password)
        }
        onSignIn?()
    }

    private func savedSessionJson() -> String? {
        Keychain.data(for: Keys.session).flatMap { String(data: $0, encoding: .utf8) }
    }

    nonisolated private static func isExpired(_ error: Error) -> Bool {
        if case WarpsignError.SessionExpired = error { return true }
        return false
    }

    /// Sign in again with the remembered password once Apple has ended the
    /// session. Apple doesn't ask for a code while it trusts this device
    /// (for about a month after the last code), so this needs nothing from
    /// the user. False if it couldn't.
    private func reauthenticate() async -> Bool {
        if let running = reauthentication { return await running.value }
        let task = Task { await signInAgain() }
        reauthentication = task
        defer { reauthentication = nil }
        return await task.value
    }

    private func signInAgain() async -> Bool {
        guard !silentSignInPaused, let appleId, let password = savedPassword else { return false }
        let account = AppleAccount.newTrusted(transport: transport, sessionJson: savedSessionJson())
        do {
            let result = try await background { try account.signIn(account: appleId, password: password) }
            guard case .signedIn(let teams) = result else {
                silentSignInPaused = true
                sessionProblem = "Apple asked for a verification code again: sign in to continue."
                return false
            }
            signedIn(account, teams: teams)
            return true
        } catch {
            switch error {
            case WarpsignError.Offline:
                // Apple couldn't be reached: the next try can try again
                break
            case WarpsignError.WrongPassword:
                // Changed since it was saved: it's no use any more
                try? Keychain.set(nil, for: Keys.password)
            default:
                // Apple refusing, or wanting something done on the web:
                // the password's kept, for the user to sign in with
                silentSignInPaused = true
            }
            sessionProblem = error.userMessage
            return false
        }
    }

    /// Run a portal call. If Apple has ended the session and the password
    /// is remembered, sign in again and run it once more.
    private func portalCall<T: Sendable>(
        _ work: @escaping @Sendable (AppleAccount) throws -> T
    ) async throws -> T {
        guard let account else { throw WarpsignError.Portal(message: "Sign in first.") }
        do {
            let result = try await background { try work(account) }
            saveSession()
            return result
        } catch {
            guard Self.isExpired(error), await reauthenticate(), let account = self.account else {
                throw handle(error)
            }
            do {
                let result = try await background { try work(account) }
                saveSession()
                return result
            } catch {
                throw handle(error)
            }
        }
    }

    /// Keep the session's latest cookies and tokens
    private func saveSession() {
        guard let account else { return }
        try? Keychain.set(Data(account.sessionJson().utf8), for: Keys.session)
    }

    /// Forget the session, its two-factor trust and the password
    func signOut() {
        endSession()
        try? Keychain.set(nil, for: Keys.session)
        try? Keychain.set(nil, for: Keys.password)
    }

    private func endSession() {
        account = nil
        teams = []
        phase = .signedOut
        portalStatus = [:]
    }

    private func handle(_ error: Error) -> Error {
        if case WarpsignError.SessionExpired(let message) = error {
            // The saved session stays: its two-factor trust lets the next
            // sign-in skip the code
            endSession()
            sessionProblem = sessionProblem ?? message
        }
        return error
    }

    // MARK: Identities

    func info(_ kind: CertificateType) -> IdentityInfo? {
        if let demo = demoInfos[kind] { return demo }
        guard let identity = identities[kind] else { return nil }
        return try? identityInfo(identity: identity.data)
    }

    var needsBackup: Bool {
        !identities.isEmpty && identities.values.contains { !$0.backedUp }
    }

    /// Make the missing certificates. Distribution is best-effort: the team
    /// may have no free slots. Returns a note about anything not created.
    func createMissingIdentities() async throws -> String? {
        guard let account, let team = selectedTeam else { return nil }
        var notes: [String] = []
        for kind in [CertificateType.development, .distribution] where identities[kind] == nil {
            do {
                let teamId = team.teamId
                let created = try await background {
                    try account.createIdentity(teamId: teamId, kind: kind)
                }
                try store(StoredIdentity(certificate: created.certificate, privateKey: created.privateKey), as: kind)
                if let serial = info(kind)?.serialNumber { portalStatus[serial] = true }
            } catch {
                let error = handle(error)
                if kind == .development { throw error }
                notes.append("No distribution certificate: \(error.userMessage). Signing works with development only; keeping original bundle IDs needs distribution.")
            }
        }
        saveSession()
        return notes.isEmpty ? nil : notes.joined(separator: "\n")
    }

    /// Restore identities from a backup `.p12`
    func importBackup(_ url: URL, password: String) async throws -> Int {
        let found = try await background { () throws -> [IdentityData] in
            let data = try withSecurityScopedAccess(to: url) { try Data(contentsOf: url) }
            return try importIdentities(p12: data, password: password)
        }
        var imported = 0
        for data in found {
            guard let kind = try identityInfo(identity: data).kind else { continue }
            try store(StoredIdentity(certificate: data.certificate, privateKey: data.privateKey, backedUp: true), as: kind)
            imported += 1
        }
        if imported == 0 {
            throw WarpsignError.Identity(message: "The file has no Apple Development or Apple Distribution identity.")
        }
        await checkIdentities()
        return imported
    }

    /// A backup `.p12` of every identity, protected by `passphrase`
    func backup(passphrase: String) async throws -> URL {
        let all = identities.values.map(\.data)
        let data = try await background {
            try exportIdentities(identities: all, password: passphrase)
        }
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent("WarpSign Signing Identity.p12")
        try data.write(to: url, options: .completeFileProtection)
        return url
    }

    func markBackedUp() {
        for (kind, var identity) in identities {
            identity.backedUp = true
            try? store(identity, as: kind)
        }
    }

    /// Forget an identity on this device. Never revokes the certificate.
    func remove(_ kind: CertificateType) {
        try? Keychain.set(nil, for: Keys.identity(kind))
        identities[kind] = nil
    }

    /// Ask the portal which identities' certificates still exist
    func checkIdentities() async {
        guard account != nil, let team = selectedTeam, !identities.isEmpty else { return }
        let all = Array(identities.values.map(\.data))
        let teamId = team.teamId
        guard let found = try? await portalCall({
            try $0.identitiesOnPortal(teamId: teamId, identities: all)
        }) else { return }
        for kind in identities.keys {
            if let serial = info(kind)?.serialNumber {
                portalStatus[serial] = found.contains(serial)
            }
        }
    }

    private func store(_ identity: StoredIdentity, as kind: CertificateType) throws {
        let data = try JSONEncoder().encode(identity)
        try Keychain.set(data, for: Keys.identity(kind))
        identities[kind] = identity
    }

    // MARK: Devices

    /// The team's iPhones and iPads, which signed apps install on
    func devices() async throws -> [DeviceInfo] {
        guard account != nil, let team = selectedTeam else { return [] }
        let teamId = team.teamId
        return try await portalCall { try $0.devices(teamId: teamId) }
    }

    /// Register a device (e.g. this iPhone, from its pairing file's UDID)
    func registerDevice(name: String, udid: String) async throws -> DeviceInfo {
        guard account != nil, let team = selectedTeam else {
            throw WarpsignError.Portal(message: "Sign in first.")
        }
        let teamId = team.teamId
        return try await portalCall { try $0.registerDevice(teamId: teamId, name: name, udid: udid) }
    }

    /// Provisioning profiles kept between signs (see profile_reuse.rs), so
    /// signing an unchanged app again makes no changes on the portal
    static var profileCacheDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Caches", isDirectory: true)
            .appendingPathComponent("Profiles", isDirectory: true)
    }

    // MARK: Entitlement names

    /// Names for entitlement keys, from the team's capabilities on the
    /// Developer Portal: fetched when signed in (and kept), otherwise the
    /// last ones fetched. With neither, only keys that are never
    /// capabilities have names.
    func entitlementNames() async -> EntitlementNames {
        var json: String?
        if account != nil, let team = selectedTeam {
            let teamId = team.teamId
            if let fetched = try? await portalCall({ try $0.capabilities(teamId: teamId) }) {
                CapabilitiesCache.save(fetched, team: teamId)
                json = fetched
            }
        }
        if json == nil, let teamId = selectedTeamId {
            json = CapabilitiesCache.load(team: teamId)
        }
        let found = json
        return (try? await background { EntitlementNames.from(capabilitiesJson: found) })
            ?? EntitlementNames()
    }

    /// Names from the team's last capabilities response, without asking
    /// the portal: for showing what an app in a source uses
    func cachedEntitlementNames() async -> EntitlementNames {
        let json = selectedTeamId.flatMap { CapabilitiesCache.load(team: $0) }
        return (try? await background { EntitlementNames.from(capabilitiesJson: json) }) ?? EntitlementNames()
    }

    // MARK: Signing

    /// Why signing can't start yet, if it can't
    func signingBlocker(for kind: CertificateType) -> String? {
        if phase != .signedIn { return "Sign in with your Apple ID in Settings first." }
        if selectedTeam == nil { return "Choose a team in Settings." }
        guard let info = info(kind) else {
            return kind == .development
                ? "Set up your signing identity in Settings first."
                : "There's no distribution certificate. Set one up in Settings, or sign with development."
        }
        if let team = info.teamId, team != selectedTeam?.teamId {
            return "This certificate is for team \(team), not the selected team."
        }
        return nil
    }

    func sign(
        ipa: URL,
        output: URL,
        kind: CertificateType,
        settings: SignSettings,
        observer: StepRecorder
    ) async throws -> SignResult {
        guard account != nil, let team = selectedTeam, let identity = identities[kind]?.data else {
            throw WarpsignError.Signing(message: signingBlocker(for: kind) ?? "Not ready to sign.")
        }
        let teamId = team.teamId
        let profileCacheDir = Self.profileCacheDir.path
        let workDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: workDir) }
        // Signed in again part way through, it starts over: the steps
        // clear their logs when they start again
        let result = try await portalCall { account in
            try? FileManager.default.removeItem(at: workDir)
            return try account.signIpa(
                teamId: teamId,
                identity: identity,
                ipaPath: ipa.path,
                outputPath: output.path,
                workDir: workDir.path,
                settings: settings,
                profileCacheDir: profileCacheDir,
                observer: observer
            )
        }
        if !result.profiles.isEmpty {
            recordPPQ(result.profiles.contains { $0.ppqCheck }, team: teamId)
        }
        // The capabilities it fetched name entitlements: keep them
        CapabilitiesCache.save(result.capabilitiesJson, team: teamId)
        return result
    }

    // MARK: Moving to a new certificate

    /// Why this kind's certificate should be moved off, if it should: it
    /// expires within 30 days, or Apple no longer lists it
    func migrationReason(_ kind: CertificateType) -> String? {
        guard let info = info(kind) else { return nil }
        if portalStatus[info.serialNumber] == false {
            return "Apple no longer lists it: it was revoked or has expired."
        }
        let expires = Date(unixTime: info.expiresAt)
        let date = expires.formatted(date: .long, time: .omitted)
        if expires < Date() { return "It expired on \(date)." }
        if expires < Date().addingTimeInterval(30 * 24 * 60 * 60) { return "It expires on \(date)." }
        if forceMigration { return "Shown by the developer option; it expires on \(date)." }
        return nil
    }

    func migration(_ kind: CertificateType) -> CertificateMigration? {
        migrations[CertificateMigration.key(kind)]
    }

    func finishMigration(_ kind: CertificateType) {
        migrations[CertificateMigration.key(kind)] = nil
        saveMigrations()
    }

    private func saveMigrations() {
        if let data = try? JSONEncoder().encode(migrations) {
            defaults.set(data, forKey: Keys.migrations)
        }
    }

    /// The team's certificates of this kind on the portal
    func portalCertificates(_ kind: CertificateType) async throws -> [PortalCertificateInfo] {
        guard let team = selectedTeam else { return [] }
        let teamId = team.teamId
        let all = try await portalCall { try $0.certificates(teamId: teamId) }
        return all.filter { $0.kind == kind }
    }

    /// Make a new certificate of this kind and sign with it from now on.
    /// The old certificate isn't touched, so apps signed with it keep
    /// working until it expires; its serial is kept to know which apps to
    /// sign again. Uses one of the team's certificate slots.
    func moveToNewCertificate(_ kind: CertificateType) async throws {
        guard account != nil, let team = selectedTeam else {
            throw WarpsignError.Portal(message: "Sign in first.")
        }
        let teamId = team.teamId
        let oldSerial = info(kind)?.serialNumber
        let created = try await portalCall { try $0.createIdentity(teamId: teamId, kind: kind) }
        try store(StoredIdentity(certificate: created.certificate, privateKey: created.privateKey), as: kind)
        if let serial = info(kind)?.serialNumber { portalStatus[serial] = true }
        if let oldSerial {
            migrations[CertificateMigration.key(kind)] = CertificateMigration(oldSerial: oldSerial, startedAt: Date())
            saveMigrations()
        }
    }

    /// Revoke a certificate on the portal. Every app signed with it stops
    /// opening at once: only after the user has explicitly chosen it.
    func revoke(_ certificate: PortalCertificateInfo) async throws {
        guard let team = selectedTeam else { return }
        let teamId = team.teamId
        let id = certificate.id
        try await portalCall { try $0.revokeCertificate(teamId: teamId, certificateId: id) }
        portalStatus[certificate.serialNumber] = false
    }
}

/// A move from one certificate to a new one, until every app is signed again
struct CertificateMigration: Codable, Equatable, Sendable {
    /// The certificate being moved off
    var oldSerial: String
    var startedAt: Date

    static func key(_ kind: CertificateType) -> String {
        kind == .development ? "development" : "distribution"
    }

    /// What signed copies call this kind of certificate
    static func copyName(_ kind: CertificateType) -> String {
        kind == .development ? "Development" : "Ad Hoc"
    }
}

