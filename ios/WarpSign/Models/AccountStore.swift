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

    @ObservationIgnored private var account: AppleAccount?
    @ObservationIgnored private let transport = URLSessionTransport()
    @ObservationIgnored private let defaults = UserDefaults.standard

    private enum Keys {
        static let appleId = "account.appleId"
        static let team = "account.team"
        static let session = "session"
        static func identity(_ kind: CertificateType) -> String {
            kind == .development ? "identity.development" : "identity.distribution"
        }
        static func ppq(_ team: String) -> String { "ppq.\(team)" }
    }

    init() {
        appleId = defaults.string(forKey: Keys.appleId)
        selectedTeamId = defaults.string(forKey: Keys.team)
        for kind in [CertificateType.development, .distribution] {
            if let data = Keychain.data(for: Keys.identity(kind)),
               let identity = try? JSONDecoder().decode(StoredIdentity.self, from: data) {
                identities[kind] = identity
            }
        }
    }

    var selectedTeam: TeamInfo? {
        teams.first { $0.teamId == selectedTeamId } ?? teams.first
    }

    var isSignedIn: Bool { phase == .signedIn && selectedTeam != nil }

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
            phase = .signedOut
            sessionProblem = error.userMessage
        }
    }

    /// Start signing in. Returns the two-factor challenge if a code is needed.
    func signIn(appleId: String, password: String) async throws -> TwoFactorInfo? {
        let account = AppleAccount(transport: transport)
        let id = appleId.trimmingCharacters(in: .whitespaces)
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
    }

    /// Keep the session's latest cookies and tokens
    private func saveSession() {
        guard let account else { return }
        try? Keychain.set(Data(account.sessionJson().utf8), for: Keys.session)
    }

    func signOut() {
        account = nil
        teams = []
        phase = .signedOut
        portalStatus = [:]
        try? Keychain.set(nil, for: Keys.session)
    }

    private func handle(_ error: Error) -> Error {
        if case WarpsignError.SessionExpired(let message) = error {
            signOut()
            sessionProblem = message
        }
        return error
    }

    // MARK: Identities

    func info(_ kind: CertificateType) -> IdentityInfo? {
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
        guard let account, let team = selectedTeam, !identities.isEmpty else { return }
        let all = Array(identities.values.map(\.data))
        let teamId = team.teamId
        do {
            let found = try await background {
                try account.identitiesOnPortal(teamId: teamId, identities: all)
            }
            for kind in identities.keys {
                if let serial = info(kind)?.serialNumber {
                    portalStatus[serial] = found.contains(serial)
                }
            }
            saveSession()
        } catch {
            _ = handle(error)
        }
    }

    private func store(_ identity: StoredIdentity, as kind: CertificateType) throws {
        let data = try JSONEncoder().encode(identity)
        try Keychain.set(data, for: Keys.identity(kind))
        identities[kind] = identity
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
        guard let account, let team = selectedTeam, let identity = identities[kind]?.data else {
            throw WarpsignError.Signing(message: signingBlocker(for: kind) ?? "Not ready to sign.")
        }
        let teamId = team.teamId
        let workDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: workDir) }
        do {
            let result = try await background {
                try account.signIpa(
                    teamId: teamId,
                    identity: identity,
                    ipaPath: ipa.path,
                    outputPath: output.path,
                    workDir: workDir.path,
                    settings: settings,
                    observer: observer
                )
            }
            if !result.profiles.isEmpty {
                recordPPQ(result.profiles.contains { $0.ppqCheck }, team: teamId)
            }
            saveSession()
            return result
        } catch {
            throw handle(error)
        }
    }
}
