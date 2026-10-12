import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The signing certificates WarpSign uses: create, back up, restore
struct IdentityView: View {
    @Environment(AccountStore.self) private var account
    @State private var working = false
    @State private var message: String?
    @State private var error: String?
    @State private var backingUp = false
    @State private var importing = false
    @State private var importURL: URL?
    @State private var importPassword = ""
    @State private var askingPassword = false
    @State private var removing: CertificateType?

    private static let p12Type = UTType("com.rsa.pkcs-12")
        ?? UTType(filenameExtension: "p12")
        ?? .data

    var body: some View {
        List {
            ForEach([CertificateType.development, .distribution], id: \.self) { kind in
                identitySection(kind)
            }

            if let message {
                Section { Label(message, systemImage: "info.circle").font(.callout) }
            }
            if let error {
                Section { ErrorRow(message: error) }
            }

            Section {
                if account.identities.count < 2 {
                    Button {
                        create()
                    } label: {
                        Label(account.identities.isEmpty ? "Create Certificates" : "Create Missing Certificate", systemImage: "plus.circle")
                    }
                    .disabled(!account.isSignedIn || working)
                }
                if !account.identities.isEmpty {
                    Button("Back Up…", systemImage: "lock.doc") { backingUp = true }
                }
                Button("Restore from Backup…", systemImage: "square.and.arrow.down") { importing = true }
                if account.isSignedIn && !account.identities.isEmpty {
                    Button("Check with Apple", systemImage: "checkmark.shield") {
                        Task { await check() }
                    }
                    .disabled(working)
                }
            } footer: {
                Text(footer)
            }
        }
        .navigationTitle("Signing Identity")
        .overlay {
            if working { ProgressView().controlSize(.large) }
        }
        .task { await check() }
        .sheet(isPresented: $backingUp) { BackupView() }
        // Any file, checked after: types depend on which apps declared them
        .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
            if case .success(let url) = result {
                importURL = url
                importPassword = ""
                askingPassword = true
            }
        }
        .alert("Backup Passphrase", isPresented: $askingPassword) {
            SecureField("Passphrase", text: $importPassword)
            Button("Cancel", role: .cancel) {}
            Button("Restore") { restore() }
        } message: {
            Text("The passphrase you saved with the backup.")
        }
        .confirmationDialog(
            "Remove this certificate from WarpSign?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                if let removing { account.remove(removing) }
            }
        } message: {
            Text("It's only removed from this device, not revoked: it keeps working anywhere else it's installed. Without a backup, WarpSign will need a new certificate.")
        }
    }

    private var footer: String {
        if !account.isSignedIn {
            return "Sign in to create certificates. Restoring a backup works without signing in."
        }
        return "WarpSign makes its own development and distribution certificates, with keys that never leave this device except in your backup. To replace one, as it nears expiry or after it's revoked, use New Certificate in Settings: it only ever revokes a certificate you choose, when there's no room for another."
    }

    @ViewBuilder
    private func identitySection(_ kind: CertificateType) -> some View {
        let title = kind == .development ? "Development" : "Distribution"
        Section(title) {
            if let info = account.info(kind) {
                LabeledContent("Name", value: info.commonName ?? "—")
                LabeledContent("Team") { Text(info.teamId ?? "—").monospaced() }
                LabeledContent("Serial") {
                    Text(info.serialNumber).font(.caption.monospaced())
                }
                LabeledContent("Expires") {
                    let expires = Date(unixTime: info.expiresAt)
                    Text(expires, format: .dateTime.day().month().year())
                        .foregroundStyle(expires < Date() ? Color.red : Color.primary)
                }
                LabeledContent("On Apple's Portal") {
                    switch account.portalStatus[info.serialNumber] {
                    case .some(true): Badge("Active", color: .green)
                    case .some(false): Badge("Revoked or Expired", color: .red)
                    case .none: Text("—").foregroundStyle(.secondary)
                    }
                }
                if account.identities[kind]?.backedUp == false {
                    Label("Not backed up", systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                }
                Button("Remove from This Device", role: .destructive) { removing = kind }
            } else {
                Text(kind == .development
                     ? "None yet. Needed for signing."
                     : "None yet. Optional: needed for Ad Hoc signing and keeping original bundle IDs.")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func create() {
        working = true
        error = nil
        message = nil
        Task {
            defer { working = false }
            do {
                message = try await account.createMissingIdentities()
                if account.needsBackup { backingUp = true }
            } catch {
                self.error = error.userMessage
            }
        }
    }

    private func restore() {
        guard let url = importURL else { return }
        let password = importPassword
        importPassword = ""
        working = true
        error = nil
        Task {
            defer { working = false }
            do {
                let count = try await account.importBackup(url, password: password)
                message = "Restored \(count) certificate\(count == 1 ? "" : "s")."
            } catch {
                self.error = error.userMessage
            }
        }
    }

    private func check() async {
        guard account.isSignedIn else { return }
        working = true
        await account.checkIdentities()
        working = false
    }
}

/// Export every identity as one passphrase-protected .p12, for a password
/// manager or Files
struct BackupView: View {
    @Environment(AccountStore.self) private var account
    @Environment(\.dismiss) private var dismiss
    @State private var passphrase = generatePassphrase()
    @State private var file: URL?
    @State private var error: String?
    @State private var copied = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text("This file and passphrase are the keys to signing as your team. Keep them somewhere safe, like a password manager, and don't share them.")
                        .font(.callout)
                }
                Section {
                    Text(passphrase)
                        .font(.title3.monospaced())
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.vertical, 6)
                    Button(copied ? "Copied" : "Copy Passphrase", systemImage: copied ? "checkmark" : "doc.on.doc") {
                        UIPasteboard.general.string = passphrase
                        copied = true
                    }
                } header: {
                    Text("1. Save the passphrase")
                }
                Section {
                    if let file {
                        ShareLink(item: file) {
                            Label("Save Backup File", systemImage: "square.and.arrow.up")
                        }
                    } else if let error {
                        ErrorRow(message: error)
                    } else {
                        ProgressView()
                    }
                } header: {
                    Text("2. Save the file")
                } footer: {
                    Text("Attach it to the passphrase's password manager entry, or save it to Files or iCloud Drive.")
                }
                Section {
                    Button("I've Saved Both") {
                        account.markBackedUp()
                        dismiss()
                    }
                    .font(.headline)
                    .disabled(file == nil)
                }
            }
            .navigationTitle("Back Up")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Later") { dismiss() }
                }
            }
            .task {
                do {
                    file = try await account.backup(passphrase: passphrase)
                } catch {
                    self.error = error.userMessage
                }
            }
            .onDisappear {
                if let file { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
            }
        }
    }
}
