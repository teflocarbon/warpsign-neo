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

    private(set) var profiles: [ProfileInfo] = []
    private(set) var profileError: String?

    /// Unpack and inspect an IPA. The work happens in Rust, off the main thread.
    func inspect(_ url: URL) async {
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
            state = .inspected(info)
        } catch {
            state = .failed(error.userMessage)
        }
        try? FileManager.default.removeItem(at: workDir)
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
