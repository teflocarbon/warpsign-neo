import SwiftUI

/// Everything about how an app gets signed, then the button
struct SignOptionsView: View {
    @Environment(AccountStore.self) private var account
    let app: LibraryApp
    @State private var options = SigningOptions.saved
    @State private var signing = false

    private var blocker: String? {
        account.signingBlocker(for: options.certificate.type)
    }

    private var teamUsesPPQ: Bool? {
        account.selectedTeam.flatMap { account.teamUsesPPQ($0.teamId) }
    }

    var body: some View {
        Form {
            Section {
                Picker("Certificate", selection: $options.certificate) {
                    ForEach(SigningOptions.CertificateChoice.allCases) { choice in
                        Text(choice.title).tag(choice)
                    }
                }
                if let team = account.selectedTeam {
                    LabeledContent("Team", value: team.name)
                }
                if let blocker {
                    Label(blocker, systemImage: "exclamationmark.circle")
                        .font(.callout)
                        .foregroundStyle(.orange)
                }
            } header: {
                Text("Signing")
            } footer: {
                Text("Development profiles allow debugging. Distribution (Ad Hoc) is needed to keep the original bundle ID. Both install only on devices registered to your team.")
            }

            Section {
                Toggle("Unique Identifiers", isOn: $options.encodeIds)
                Toggle("Patch IDs in Binaries", isOn: $options.patchIds)
                Toggle("Keep Original Bundle ID", isOn: $options.forceOriginalId)
                if options.forceOriginalId {
                    originalIdWarning
                }
            } header: {
                Text("Identifiers")
            } footer: {
                Text("Unique identifiers stop the signed app clashing with the original. Turn them off only for your own apps. Keeping the original bundle ID can fix push notifications.")
            }

            Section("App") {
                TextField("Name (\(app.displayName))", text: $options.bundleName)
                Toggle("Support All Devices", isOn: $options.patchAllDevices)
                Toggle("File Sharing", isOn: $options.patchFileSharing)
                Toggle("ProMotion (120 Hz)", isOn: $options.patchPromotion)
                Toggle("Fullscreen on iPad", isOn: $options.patchFullscreen)
                Toggle("All Orientations", isOn: $options.patchOrientation)
                Toggle("Game Mode", isOn: $options.patchGameMode)
                Toggle("Remove URL Schemes", isOn: $options.removeUrlSchemes)
            }

            Section {
                Picker("Status Bar", selection: $options.statusBar) {
                    ForEach(SigningOptions.StatusBarChoice.allCases) { choice in
                        Text(choice.title).tag(choice)
                    }
                }
                Picker("Appearance", selection: $options.interfaceStyle) {
                    ForEach(SigningOptions.StyleChoice.allCases) { choice in
                        Text(choice.title).tag(choice)
                    }
                }
            } header: {
                Text("Display")
            } footer: {
                Text("Orientations and ProMotion can upset apps that don't expect them.")
            }

            Section {
                Toggle("Allow Debugging", isOn: $options.patchDebug)
            } footer: {
                Text("Adds get-task-allow, so debuggers can attach. Needs a development certificate.")
            }

            if app.isEncrypted {
                Section {
                    Label("This IPA is still encrypted: it will crash on launch once signed.", systemImage: "lock.fill")
                        .foregroundStyle(.red)
                        .font(.callout)
                }
            }

            Section {
                Button {
                    options.save()
                    signing = true
                } label: {
                    Text("Sign \(app.displayName)")
                        .frame(maxWidth: .infinity)
                        .font(.headline)
                }
                .buttonStyle(.borderedProminent)
                .listRowInsets(EdgeInsets())
                .disabled(blocker != nil)
            }
            .listRowBackground(Color.clear)
        }
        .navigationTitle("Sign")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(isPresented: $signing) {
            SigningView(app: app, options: options)
        }
    }

    @ViewBuilder
    private var originalIdWarning: some View {
        switch teamUsesPPQ {
        case .some(true):
            Label("Your team's profiles have PPQ checking: apps check in with Apple on first launch, and an original bundle ID can get your Apple ID flagged. WarpSign will refuse to sign like this.", systemImage: "exclamationmark.octagon.fill")
                .font(.callout)
                .foregroundStyle(.red)
        case .some(false):
            Label("Your team's profiles don't have PPQ checking.", systemImage: "checkmark.shield")
                .font(.callout)
                .foregroundStyle(.green)
        case .none:
            Label("WarpSign checks your team's profiles for PPQ before signing, and stops if keeping the original ID could get your Apple ID flagged.", systemImage: "shield.lefthalf.filled")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }
}
