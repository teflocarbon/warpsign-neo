import PhotosUI
import SwiftUI
import UIKit

/// Everything about how an app gets signed, then the button
struct SignOptionsView: View {
    @Environment(AccountStore.self) private var account
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    /// Set when the view appears: this app's own last options
    @State private var options = SigningOptions()
    @State private var loaded = false
    @State private var signing = false
    @State private var iconItem: PhotosPickerItem?
    /// The chosen icon, as PNG
    @State private var icon: Data?

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
                if let preview = previewBundleId {
                    LabeledContent("Signs As") {
                        Text(preview)
                            .font(.caption.monospaced())
                            .multilineTextAlignment(.trailing)
                    }
                    .contextMenu { CopyMenu(title: "Bundle ID", value: preview) }
                    if let last = lastBundleId, last != preview {
                        Label {
                            Text("Last signed as \(last). This gives it a different bundle ID, so it installs as a separate app, and doesn't get the other one's data.")
                        } icon: {
                            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                        }
                        .font(.callout)
                        if let lastOptions = library.lastSigningOptions(of: app) {
                            Button("Use Its Last Options", systemImage: "arrow.uturn.backward") {
                                options = lastOptions
                            }
                        }
                    }
                }
            } header: {
                Text("Identifiers")
            } footer: {
                Text("Unique identifiers stop the signed app clashing with the original. Turn them off only for your own apps. Keeping the original bundle ID can fix push notifications.")
            }

            Section("App") {
                TextField("Name (\(app.displayName))", text: $options.bundleName)
                HStack {
                    PhotosPicker(selection: $iconItem, matching: .images) {
                        Label(icon == nil ? "Change Icon…" : "Icon", systemImage: "photo")
                    }
                    .buttonStyle(.borderless)
                    Spacer()
                    if let icon, let image = UIImage(data: icon) {
                        AppIconView(image: image, size: 36)
                        Button("Reset", systemImage: "xmark.circle.fill") {
                            self.icon = nil
                            iconItem = nil
                        }
                        .labelStyle(.iconOnly)
                        .foregroundStyle(.secondary)
                        .buttonStyle(.borderless)
                    }
                }
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
        .onAppear {
            // This app's own options, once: never another app's
            guard !loaded else { return }
            loaded = true
            options = SigningOptions.starting(for: app, in: library)
            // And the icon it was last signed with, if it had its own
            if icon == nil { icon = library.customIcon(of: app) }
        }
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(isPresented: $signing) {
            SigningView(app: app, options: options, icon: icon)
        }
        .onChange(of: iconItem) { _, item in
            guard let item else { return }
            Task {
                // Photos can be HEIC: hand warpsign-core a PNG
                if let data = try? await item.loadTransferable(type: Data.self),
                   let png = UIImage(data: data)?.pngData() {
                    icon = png
                }
            }
        }
    }

    /// The bundle ID this sign gives the app, as the signer works it out
    private var previewBundleId: String? {
        guard let team = account.selectedTeam?.teamId else { return nil }
        return signedBundleId(
            bundleId: app.bundleId,
            teamId: team,
            encodeIds: options.encodeIds,
            forceOriginalId: options.forceOriginalId
        )
    }

    /// What its newest signed copy (for the team signing now) was signed as
    private var lastBundleId: String? {
        let team = account.selectedTeam?.teamId
        return app.signedCopies
            .filter { team == nil || $0.teamId == team }
            .max(by: { $0.date < $1.date })?
            .bundleId
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
