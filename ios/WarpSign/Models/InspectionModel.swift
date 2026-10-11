import Foundation
import Observation

/// Inspecting IPAs and checking provisioning profiles: the diagnostics
@MainActor
@Observable
final class InspectionModel {
    enum State {
        case idle
        case inspecting
        case inspected(AppInfo)
        case failed(String)
    }

    private(set) var state: State = .idle
    /// Steps of the current or last inspection
    private(set) var steps: [StepState] = []
    /// Names for entitlement keys, from the portal (see
    /// `AccountStore.entitlementNames`)
    private(set) var entitlementNames = EntitlementNames()

    /// The IPA (path and modification date) `state` is about, so coming
    /// back to the screen doesn't unpack it again
    private var inspectedKey: String?
    /// The team `entitlementNames` were fetched for, once they have been
    private var namesTeam: String??

    private(set) var profiles: [ProfileInfo] = []
    private(set) var profileError: String?

    /// Unpack and inspect an IPA, unless this one already is (or is being)
    /// inspected. The work happens in Rust, off the main thread.
    func inspect(_ url: URL) async {
        let key = Self.key(for: url)
        if key == inspectedKey {
            switch state {
            case .inspected, .inspecting: return
            case .idle, .failed: break
            }
        }
        inspectedKey = key
        state = .inspecting
        steps = [StepState](pending: inspectSteps())
        let recorder = StepRecorder { [weak self] event in
            self?.steps.apply(event)
        }
        let workDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        do {
            let info = try await background {
                try withSecurityScopedAccess(to: url) {
                    try inspectIpa(ipaPath: url.path, workDir: workDir.path, observer: recorder)
                }
            }
            // Unless another IPA's inspection started meanwhile
            if inspectedKey == key { state = .inspected(info) }
        } catch {
            if inspectedKey == key { state = .failed(error.userMessage) }
        }
        try? FileManager.default.removeItem(at: workDir)
    }

    /// Fetch names for entitlement keys, once per team
    func loadEntitlementNames(from account: AccountStore) async {
        let team = account.selectedTeamId
        if namesTeam == .some(team) { return }
        entitlementNames = await account.entitlementNames()
        namesTeam = .some(team)
    }

    private static func key(for url: URL) -> String {
        let modified = (try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate]) as? Date
        return "\(url.path)|\(modified?.timeIntervalSince1970 ?? 0)"
    }

    /// Read a provisioning profile and add it to the list
    func addProfile(_ url: URL) async {
        do {
            let profile = try await background {
                try withSecurityScopedAccess(to: url) { try readProfile(path: url.path) }
            }
            profiles.removeAll { $0.uuid != nil && $0.uuid == profile.uuid }
            profiles.insert(profile, at: 0)
            profileError = nil
        } catch {
            profileError = error.userMessage
        }
    }

    func removeProfiles(at offsets: IndexSet) {
        profiles.remove(atOffsets: offsets)
    }
}
