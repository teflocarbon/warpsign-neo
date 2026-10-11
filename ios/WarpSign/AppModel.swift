import Foundation
import Observation

@MainActor
@Observable
final class AppModel {
    enum State {
        case idle
        case inspecting(fileName: String)
        case inspected(AppInfo)
        case failed(String)
    }

    private(set) var state: State = .idle
    /// Steps of the current or last operation
    private(set) var steps: [StepState] = []

    private(set) var profiles: [ProfileInfo] = []
    private(set) var profileError: String?

    /// Unpack and inspect an IPA. The work happens in Rust, off the main thread.
    func inspect(_ url: URL) async {
        state = .inspecting(fileName: url.lastPathComponent)
        steps = [StepState](pending: inspectSteps())
        let recorder = StepRecorder { [weak self] event in
            self?.steps.apply(event)
        }
        do {
            let info = try await Task.detached(priority: .userInitiated) {
                try Self.inspectCopy(of: url, observer: recorder)
            }.value
            state = .inspected(info)
        } catch {
            state = .failed(error.userMessage)
        }
    }

    nonisolated private static func inspectCopy(
        of url: URL,
        observer: StepRecorder
    ) throws -> AppInfo {
        try withAccess(to: url) {
            let workDir = FileManager.default.temporaryDirectory
                .appendingPathComponent(UUID().uuidString, isDirectory: true)
            return try inspectIpa(ipaPath: url.path, workDir: workDir.path, observer: observer)
        }
    }

    /// Read a provisioning profile and add it to the list
    func addProfile(_ url: URL) async {
        do {
            let profile = try await Task.detached(priority: .userInitiated) {
                try Self.withAccess(to: url) { try readProfile(path: url.path) }
            }.value
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

    /// Files picked outside the app's sandbox need explicit access
    nonisolated private static func withAccess<T>(
        to url: URL,
        _ work: () throws -> T
    ) throws -> T {
        let accessing = url.startAccessingSecurityScopedResource()
        defer {
            if accessing { url.stopAccessingSecurityScopedResource() }
        }
        return try work()
    }
}

extension Error {
    /// The message from warpsign-core, rather than the generated debug description
    var userMessage: String {
        guard let error = self as? WarpsignError else { return localizedDescription }
        switch error {
        case .Ipa(let message), .Inspect(let message), .Profile(let message):
            return message
        }
    }
}
