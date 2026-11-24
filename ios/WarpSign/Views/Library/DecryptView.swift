import SwiftUI

/// Decrypt an encrypted library app, either with a key (DecryptionKeys),
/// or on this device (OnDeviceDecrypt): make the helper, install it, open
/// it. Either way the decrypted app goes into the library.
struct DecryptView: View {
    @Environment(AccountStore.self) private var account
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    @State private var steps = [StepState](pending: signSteps())
    @State private var keyDecrypting = false
    @State private var keyProgress = 0.0
    @State private var keyDone: String?
    @State private var keyError: String?

    private var decrypt: OnDeviceDecrypt { .shared }

    /// This app's decrypt, when it's the one going
    private var phase: OnDeviceDecrypt.Phase? {
        decrypt.appId == app.id ? decrypt.phase : nil
    }

    private var keys: DecryptionKeys { .shared }

    var body: some View {
        List {
            keySection

            Section {
                Label {
                    Text("\(app.displayName) is decrypted on this \(UIDevice.current.model) with its App Store licence, so the Apple ID that got it has to be signed in to the App Store here. WarpSign puts it in a small helper app, which you install and open: it decrypts \(app.displayName) and hands it back to your library.")
                } icon: {
                    Image(systemName: "lock.open.fill").foregroundStyle(.tint)
                }
                .font(.callout)
            } header: {
                Text("On This \(UIDevice.current.model)")
            }

            switch phase {
            case nil:
                Section {
                    Button("Make the Helper", systemImage: "hammer") { start() }
                        .disabled(account.signingBlocker(for: .development) != nil)
                } footer: {
                    if let blocker = account.signingBlocker(for: .development) {
                        Text(blocker)
                    } else {
                        Text("Signed with your development certificate. Another decrypt going is stopped.")
                    }
                }
            case .preparing:
                Section {
                    HStack(spacing: 12) {
                        ProgressView()
                        Text("Making the helper…")
                    }
                }
            case .ready(let helper):
                Section {
                    NavigationLink {
                        InstallView(ipa: helper, name: "Decrypt")
                    } label: {
                        Label("1. Install the Helper", systemImage: "arrow.down.app")
                    }
                    Button("2. Open the Helper", systemImage: "arrow.up.forward.app") {
                        Task { await decrypt.openHelper() }
                    }
                } footer: {
                    if decrypt.notInstalled {
                        Text("The helper isn't installed yet: install it, then open it.")
                            .foregroundStyle(.red)
                    } else {
                        Text("It's called Decrypt on the Home Screen. Open it here: it decrypts \(app.displayName) and comes back to WarpSign.")
                    }
                }
            case .waitingForHelper:
                Section {
                    HStack(spacing: 12) {
                        ProgressView()
                        Text("Decrypting in the helper…")
                    }
                    Button("Open the Helper Again", systemImage: "arrow.clockwise") {
                        decrypt.retry()
                        Task { await decrypt.openHelper() }
                    }
                } footer: {
                    Text("If it stopped with an error, it says why. Most often: the Apple ID that got \(app.displayName) isn't the one signed in to the App Store on this \(UIDevice.current.model).")
                }
            case .receiving(let name):
                Section {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Adding \(name), decrypted, to your library…")
                    }
                }
            case .received(let name):
                Section {
                    Label("\(name), decrypted, is in your library.", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    Button("Done") {
                        decrypt.finish()
                    }
                } footer: {
                    Text("You can delete the Decrypt helper from the Home Screen. Sign the decrypted copy, not this one.")
                }
            case .failed(let message):
                Section {
                    ErrorRow(message: message)
                    Button("Try Again", systemImage: "arrow.clockwise") { start() }
                }
            }

            if phase == .preparing || !steps.allSatisfy({ $0.status == .pending }) {
                Section {
                    StepListView(steps: steps)
                } header: {
                    Text("Signing the Helper")
                } footer: {
                    CopyLogButton(steps: steps)
                }
            }
        }
        .navigationTitle("Decrypt")
        .navigationBarTitleDisplayMode(.inline)
    }

    /// Decrypting with a key: seconds, nothing to install
    @ViewBuilder private var keySection: some View {
        Section {
            if keys.keys.isEmpty {
                NavigationLink {
                    DecryptionKeysView()
                } label: {
                    Label("Add a Decryption Key", systemImage: "key")
                }
            } else if let keyDone {
                Label("Decrypted with \(keyDone), and added to your library.", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            } else if keyDecrypting {
                ProgressView(value: keyProgress) {
                    Text("Decrypting…")
                }
            } else {
                Button("Decrypt with Your Key", systemImage: "key") { decryptWithKey() }
                if let keyError {
                    ErrorRow(message: keyError)
                }
            }
        } header: {
            Text("With a Key")
        } footer: {
            if keys.keys.isEmpty {
                Text("With the key for the Apple ID that got \(app.displayName), WarpSign decrypts it right here in seconds, with no helper.")
            } else if keyDone == nil {
                Text("Takes seconds, with nothing to install. Needs the key for the Apple ID that got \(app.displayName).")
            }
        }
    }

    private func decryptWithKey() {
        keyDecrypting = true
        keyProgress = 0
        keyError = nil
        let recorder = StepRecorder { event in
            if case .progress(_, let fraction) = event { keyProgress = fraction }
        }
        Task {
            do {
                keyDone = try await keys.decrypt(app, library: library, observer: recorder)
            } catch {
                keyError = error.userMessage
            }
            keyDecrypting = false
        }
    }

    private func start() {
        steps = [StepState](pending: signSteps())
        let recorder = StepRecorder { event in steps.apply(event) }
        Task { await decrypt.prepare(app, account: account, observer: recorder) }
    }
}
