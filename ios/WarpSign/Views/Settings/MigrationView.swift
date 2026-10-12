import SwiftUI

/// Moving to a new certificate: make it while the old one still works, back
/// it up, then sign each app again with it. Revoking is offered only when
/// the team has no room for another, and only for a certificate the user
/// picks, after saying what that breaks.
struct MigrationView: View {
    @Environment(AccountStore.self) private var account
    @Environment(LibraryStore.self) private var library
    let kind: CertificateType

    @State private var working = false
    @State private var error: String?
    @State private var backingUp = false
    /// The team's certificates of this kind, once making a new one failed
    @State private var certificates: [PortalCertificateInfo]?
    @State private var revoking: PortalCertificateInfo?
    @State private var revoked: String?

    private var name: String { kind == .development ? "development" : "distribution" }

    var body: some View {
        List {
            if let migration = account.migration(kind) {
                movingSections(migration)
            } else {
                introSections
            }
        }
        .navigationTitle("New Certificate")
        .overlay {
            if working { ProgressView().controlSize(.large) }
        }
        .sheet(isPresented: $backingUp) { BackupView() }
        .confirmationDialog(
            revoking.map { "Revoke \($0.displayName ?? $0.name)?" } ?? "",
            isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }),
            titleVisibility: .visible,
            presenting: revoking
        ) { certificate in
            Button("Revoke, and Break Those Apps", role: .destructive) { revoke(certificate) }
        } message: { certificate in
            Text(revokeWarning(certificate))
        }
    }

    // MARK: Before

    @ViewBuilder
    private var introSections: some View {
        Section {
            if let reason = account.migrationReason(kind) {
                Label(reason, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            }
            Text("WarpSign makes a new \(name) certificate and signs with it from now on. Your current one isn't touched: apps signed with it keep working until it expires. Then you sign each of them again with the new one, which WarpSign lists for you.")
        } header: {
            Text("Move to a New Certificate")
        }

        if let info = account.info(kind) {
            Section("Current Certificate") {
                LabeledContent("Name", value: info.commonName ?? "—")
                LabeledContent("Expires") {
                    Text(Date(unixTime: info.expiresAt), format: .dateTime.day().month().year())
                }
                let apps = appsOn(info.serialNumber, before: Date())
                LabeledContent("Apps Signed With It", value: "\(apps.count)")
            }
        }

        Section {
            Button {
                makeNew()
            } label: {
                Label("Make a New \(kind == .development ? "Development" : "Distribution") Certificate", systemImage: "plus.circle")
            }
            .disabled(working || !account.isSignedIn)
            if let error {
                ErrorRow(message: error)
            }
        } footer: {
            Text("Uses one of your team's certificate slots.")
        }

        if let revoked {
            Section { Label(revoked, systemImage: "checkmark.circle").foregroundStyle(.green) }
        }

        if let certificates {
            Section {
                ForEach(certificates, id: \.id) { certificate in
                    Button(role: .destructive) {
                        revoking = certificate
                    } label: {
                        certificateRow(certificate)
                    }
                }
            } header: {
                Text("Make Room")
            } footer: {
                Text("If Apple says there's no room for another certificate, one of these has to be revoked first. Revoking is immediate and can't be undone: every app signed with that certificate stops opening, on every device.")
            }
        }
    }

    private func certificateRow(_ certificate: PortalCertificateInfo) -> some View {
        let apps = appsOn(certificate.serialNumber, before: Date())
        let isCurrent = account.info(kind)?.serialNumber.caseInsensitiveCompare(certificate.serialNumber) == .orderedSame
        return VStack(alignment: .leading, spacing: 3) {
            Text(certificate.displayName ?? certificate.name)
                .foregroundStyle(.primary)
            Group {
                if let expires = certificate.expirationDate?.prefix(10) {
                    Text("Expires \(String(expires)) · serial …\(String(certificate.serialNumber.suffix(6)))")
                }
                Text(isCurrent ? "WarpSign's current one · \(apps.count) apps use it" : "\(apps.count) apps in WarpSign use it")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    // MARK: After

    @ViewBuilder
    private func movingSections(_ migration: CertificateMigration) -> some View {
        let remaining = appsOn(migration.oldSerial, before: migration.startedAt)
        Section {
            Label("New \(name) certificate made", systemImage: "checkmark.seal.fill")
                .foregroundStyle(.green)
            if account.identities[kind]?.backedUp == false {
                Button("Back Up the New Certificate…", systemImage: "lock.doc") { backingUp = true }
            }
        } footer: {
            Text("Back it up like the last one: it's what signs your apps from now on.")
        }

        Section {
            if remaining.isEmpty {
                Label("Every app is signed with the new certificate.", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                Button("Done") { account.finishMigration(kind) }
            } else {
                ForEach(remaining) { app in
                    NavigationLink {
                        AppDetailView(appId: app.id)
                    } label: {
                        HStack(spacing: 12) {
                            AppIconView(image: library.icon(for: app.id))
                            Text(app.displayName).lineLimit(1)
                        }
                    }
                }
            }
        } header: {
            Text(remaining.isEmpty ? "Apps" : "Sign These Again (\(remaining.count))")
        } footer: {
            if !remaining.isEmpty {
                Text("Their newest signed copy is from the old certificate. Sign and install each one again; they keep working until the old certificate expires or is revoked.")
            }
        }

        Section {
            Button("Stop Tracking This Move", role: .destructive) { account.finishMigration(kind) }
        } footer: {
            Text("Forgets the list above. The certificates aren't changed.")
        }
    }

    // MARK: Actions

    private func appsOn(_ serial: String, before: Date) -> [LibraryApp] {
        library.apps(signedWith: serial, certificate: CertificateMigration.copyName(kind), before: before)
    }

    private func makeNew() {
        working = true
        error = nil
        revoked = nil
        Task {
            defer { working = false }
            do {
                try await account.moveToNewCertificate(kind)
                certificates = nil
                backingUp = true
            } catch {
                self.error = error.userMessage
                // Most often: no room for another. Show what could be revoked.
                certificates = try? await account.portalCertificates(kind)
            }
        }
    }

    private func revokeWarning(_ certificate: PortalCertificateInfo) -> String {
        let apps = appsOn(certificate.serialNumber, before: Date())
        var text = "Every app signed with this certificate stops opening immediately, on every device. This can't be undone."
        if !apps.isEmpty {
            let names = apps.prefix(5).map(\.displayName).joined(separator: ", ")
            let more = apps.count > 5 ? " and \(apps.count - 5) more" : ""
            text += "\n\nIn WarpSign, that's \(names)\(more)."
        }
        return text
    }

    private func revoke(_ certificate: PortalCertificateInfo) {
        working = true
        error = nil
        Task {
            defer { working = false }
            do {
                try await account.revoke(certificate)
                revoked = "Revoked \(certificate.displayName ?? certificate.name). Make the new certificate now."
                certificates = try? await account.portalCertificates(kind)
            } catch {
                self.error = error.userMessage
            }
        }
    }
}

/// Shown above the library, and in Settings, while a certificate needs
/// moving off or a move is under way
struct MigrationBanner: View {
    @Environment(AccountStore.self) private var account

    var body: some View {
        ForEach([CertificateType.development, .distribution], id: \.self) { kind in
            if account.migration(kind) != nil || account.migrationReason(kind) != nil {
                Section {
                    NavigationLink {
                        MigrationView(kind: kind)
                    } label: {
                        Label {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(account.migration(kind) != nil
                                     ? "Finish moving to your new certificate"
                                     : "Move to a new \(kind == .development ? "development" : "distribution") certificate")
                                    .font(.headline)
                                if let reason = account.migrationReason(kind), account.migration(kind) == nil {
                                    Text(reason).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        } icon: {
                            Image(systemName: "arrow.triangle.2.circlepath.circle.fill")
                                .foregroundStyle(.orange)
                        }
                    }
                }
            }
        }
    }
}
