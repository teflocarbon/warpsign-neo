import SwiftUI

struct AppDetailView: View {
    @Environment(LibraryStore.self) private var library
    @Environment(\.dismiss) private var dismiss
    let appId: UUID
    @State private var confirmingDelete = false

    var body: some View {
        if let app = library.app(appId) {
            content(app)
        } else {
            ContentUnavailableView("App Removed", systemImage: "trash")
        }
    }

    private func content(_ app: LibraryApp) -> some View {
        List {
            Section {
                HStack(spacing: 16) {
                    AppIconView(image: library.icon(for: app.id), size: 84)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(app.displayName)
                            .font(.title2.weight(.semibold))
                        Text(app.bundleId)
                            .font(.footnote.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                        Text("Version \(app.versionText)")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 6)
                NavigationLink {
                    SignOptionsView(app: app)
                } label: {
                    Label("Sign", systemImage: "signature")
                        .font(.headline)
                }
            }

            if app.isEncrypted {
                Section {
                    Label {
                        Text("This app is still encrypted (as downloaded from the App Store). It will install once signed, but crash on launch. Use a decrypted IPA.")
                    } icon: {
                        Image(systemName: "lock.fill").foregroundStyle(.red)
                    }
                    .font(.callout)
                }
            }

            if !app.signedCopies.isEmpty {
                Section {
                    ForEach(app.signedCopies) { copy in
                        SignedCopyRow(app: app, copy: copy)
                    }
                    .onDelete { offsets in
                        for index in offsets {
                            library.deleteSignedCopy(app.signedCopies[index], of: app)
                        }
                    }
                } header: {
                    Text("Signed")
                } footer: {
                    Text("Share a signed IPA to install it, for example with Apple Configurator or by AirDrop to a Mac.")
                }
            }

            Section("Details") {
                LabeledContent("Version", value: app.version ?? "—")
                LabeledContent("Build", value: app.build ?? "—")
                LabeledContent("Requires", value: app.minimumOS.map { "iOS \($0)" } ?? "—")
                LabeledContent("Size", value: ByteCountFormatter.file(app.fileSize))
                LabeledContent("Installed Size", value: ByteCountFormatter.file(app.installedSize))
                LabeledContent("Extensions", value: "\(app.extensionCount)")
                LabeledContent("Encryption", value: app.encryption.capitalized)
                if let origin = app.origin {
                    LabeledContent("From", value: origin)
                }
                LabeledContent("Added") {
                    Text(app.addedAt, format: .dateTime.day().month().year())
                }
            }

            Section {
                NavigationLink {
                    InspectionView(ipa: LibraryStore.ipaURL(of: app.id))
                } label: {
                    Label("Components and Entitlements", systemImage: "shippingbox")
                }
                ShareLink(item: LibraryStore.ipaURL(of: app.id)) {
                    Label("Share Original IPA", systemImage: "square.and.arrow.up")
                }
                Button("Remove from Library", systemImage: "trash", role: .destructive) {
                    confirmingDelete = true
                }
            }
        }
        .navigationTitle(app.displayName)
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("Remove \(app.displayName)?", isPresented: $confirmingDelete, titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                library.delete(app)
                dismiss()
            }
        } message: {
            Text("The IPA and its signed copies are deleted from WarpSign.")
        }
    }
}

private struct SignedCopyRow: View {
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    let copy: SignedCopy

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(copy.date, format: .dateTime.day().month().hour().minute())
                Text(copy.bundleId)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                HStack(spacing: 6) {
                    Badge(copy.certificate, color: .blue)
                    if copy.ppqCheck {
                        Badge("PPQ", color: .orange)
                    }
                    if let expires = copy.profileExpires {
                        Text("Expires \(expires, format: .dateTime.day().month())")
                            .font(.caption2)
                            .foregroundStyle(expires < Date() ? Color.red : Color.secondary)
                    }
                }
            }
            Spacer()
            ShareLink(item: library.signedURL(copy, of: app)) {
                Image(systemName: "square.and.arrow.up")
            }
            .buttonStyle(.borderless)
        }
    }
}
