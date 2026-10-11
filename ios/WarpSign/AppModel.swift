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

    /// Unpack and inspect an IPA. The work happens in Rust, off the main thread.
    func inspect(_ url: URL) async {
        state = .inspecting(fileName: url.lastPathComponent)
        do {
            let info = try await Task.detached(priority: .userInitiated) {
                try Self.inspectCopy(of: url)
            }.value
            state = .inspected(info)
        } catch {
            state = .failed(error.userMessage)
        }
    }

    nonisolated private static func inspectCopy(of url: URL) throws -> AppInfo {
        // Files picked outside the app's sandbox need explicit access
        let accessing = url.startAccessingSecurityScopedResource()
        defer {
            if accessing { url.stopAccessingSecurityScopedResource() }
        }
        let workDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        return try inspectIpa(ipaPath: url.path, workDir: workDir.path)
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
