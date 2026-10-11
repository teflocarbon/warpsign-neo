import SwiftUI

/// Signs an app, showing each step as it happens, then the result
struct SigningView: View {
    @Environment(AccountStore.self) private var account
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    let options: SigningOptions
    /// New icon as PNG, if one was chosen
    var icon: Data?

    private enum Phase {
        case signing
        case done(SignResult, SignedCopy)
        case failed(String)
    }

    @State private var phase = Phase.signing
    @State private var steps = [StepState](pending: signSteps())
    @State private var started = false

    var body: some View {
        List {
            Section { header }

            if case .done(let result, let copy) = phase {
                Section {
                    InstallButton(
                    ipa: library.signedURL(copy, of: app),
                    name: app.displayName,
                    icon: LibraryStore.folder(of: app.id).appendingPathComponent("icon.png")
                )
                    ShareLink(item: library.signedURL(copy, of: app)) {
                        Label("Share Signed IPA", systemImage: "square.and.arrow.up")
                    }
                } footer: {
                    Text("Installs on the devices registered to your team.")
                }
                if !result.warnings.isEmpty {
                    Section("Warnings") {
                        ForEach(result.warnings, id: \.self) { warning in
                            Label(warning, systemImage: "exclamationmark.triangle")
                                .foregroundStyle(.orange)
                                .font(.callout)
                        }
                    }
                }
                Section("Profiles") {
                    ForEach(result.profiles, id: \.bundlePath) { profile in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack {
                                Text(profile.bundlePath.isEmpty ? "App" : profile.bundlePath)
                                Spacer()
                                Badge(profile.ppqCheck ? "PPQ on" : "PPQ off", color: profile.ppqCheck ? .orange : .green)
                            }
                            Text(profile.bundleId)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                            if let expires = profile.expiresAt {
                                Text("Expires \(Date(unixTime: expires), format: .dateTime.day().month().year())")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }

            Section {
                StepListView(steps: steps)
            } header: {
                Text("Steps")
            } footer: {
                CopyLogButton(steps: steps)
            }
        }
        .navigationTitle(app.displayName)
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(isSigning)
        .interactiveDismissDisabled(isSigning)
        .task {
            guard !started else { return }
            started = true
            await sign()
        }
    }

    private var isSigning: Bool {
        if case .signing = phase { return true }
        return false
    }

    @ViewBuilder
    private var header: some View {
        switch phase {
        case .signing:
            HStack(spacing: 12) {
                ProgressView()
                VStack(alignment: .leading) {
                    Text("Signing…").font(.headline)
                    Text("Keep WarpSign open until it's done.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        case .done(let result, _):
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Signed").font(.headline)
                    Text(result.bundleId)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            } icon: {
                Image(systemName: "checkmark.seal.fill").foregroundStyle(.green)
            }
        case .failed(let message):
            ErrorRow(message: message)
        }
    }

    private func sign() async {
        let (output, draft) = library.newSignedCopy(of: app)
        let recorder = StepRecorder { event in
            steps.apply(event)
        }
        do {
            let result = try await account.sign(
                ipa: LibraryStore.ipaURL(of: app.id),
                output: output,
                kind: options.certificate.type,
                settings: options.settings(icon: icon),
                observer: recorder
            )
            var copy = draft
            copy.bundleId = result.bundleId
            copy.teamId = account.selectedTeam?.teamId ?? ""
            copy.certificate = options.certificate == .development ? "Development" : "Ad Hoc"
            copy.ppqCheck = result.profiles.contains { $0.ppqCheck }
            copy.profileExpires = result.profiles
                .compactMap { $0.expiresAt.map(Date.init(unixTime:)) }
                .min()
            library.addSignedCopy(copy, to: app)
            phase = .done(result, copy)
        } catch {
            try? FileManager.default.removeItem(at: output.deletingLastPathComponent())
            phase = .failed(error.userMessage)
        }
    }
}
