import Foundation

/// Run blocking warpsign-core work off the main thread
func background<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await Task.detached(priority: .userInitiated) {
        try work()
    }.value
}

/// `background`, for views, where that name means SwiftUI's modifier
func runInBackground<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await background(work)
}

/// Files picked outside the app's sandbox need explicit access
func withSecurityScopedAccess<T>(to url: URL, _ work: () throws -> T) rethrows -> T {
    let accessing = url.startAccessingSecurityScopedResource()
    defer {
        if accessing { url.stopAccessingSecurityScopedResource() }
    }
    return try work()
}

extension Error {
    /// The message from warpsign-core, rather than the generated debug description
    var userMessage: String {
        if let error = self as? WarpsignError {
            switch error {
            case .Ipa(let message), .Inspect(let message), .Profile(let message),
                 .Source(let message), .Auth(let message), .IncorrectCode(let message),
                 .SessionExpired(let message), .Portal(let message),
                 .Identity(let message), .Signing(let message),
                 .DeviceUnreachable(let message), .Device(let message):
                return message
            }
        }
        return localizedDescription
    }
}

extension ByteCountFormatter {
    static func file(_ bytes: UInt64) -> String {
        string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }
}

extension Date {
    init(unixTime: Int64) {
        self.init(timeIntervalSince1970: TimeInterval(unixTime))
    }
}
