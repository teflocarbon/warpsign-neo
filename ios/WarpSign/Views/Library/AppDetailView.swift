import SwiftUI
import UIKit

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

            AppUpdatesSection(app: app)

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
                        NavigationLink {
                            SignedCopyView(app: app, copy: copy)
                        } label: {
                            SignedCopyRow(app: app, copy: copy)
                        }
                    }
                    .onDelete { offsets in
                        for index in offsets {
                            library.deleteSignedCopy(app.signedCopies[index], of: app)
                        }
                    }
                } header: {
                    Text("Signed")
                } footer: {
                    Text("Install a signed copy on this device, or share it.")
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
                LabeledContent("From", value: app.origin ?? "Imported File")
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
        }
    }
}

/// One signed copy: install or share it, and see how it was signed
private struct SignedCopyView: View {
    @Environment(LibraryStore.self) private var library
    let app: LibraryApp
    let copy: SignedCopy

    var body: some View {
        let record = library.signingRecord(for: copy, of: app)
        List {
            Section {
                InstallButton(
                    ipa: library.signedURL(copy, of: app),
                    name: app.displayName,
                    icon: LibraryStore.folder(of: app.id).appendingPathComponent("icon.png"),
                    bundleId: copy.bundleId
                )
                ShareLink(item: library.signedURL(copy, of: app)) {
                    Label("Share Signed IPA", systemImage: "square.and.arrow.up")
                }
            }
            if let record {
                signingReportSection(record)
            }
            Section {
                LabeledContent("Signed") {
                    Text(copy.date, format: .dateTime.day().month().year().hour().minute())
                }
                LabeledContent("Bundle ID") {
                    Text(copy.bundleId).font(.caption.monospaced()).textSelection(.enabled)
                }
                LabeledContent("Team") { Text(copy.teamId).monospaced() }
                LabeledContent("Certificate", value: copy.certificate)
                LabeledContent("PPQ Checking", value: copy.ppqCheck ? "On" : "Off")
                if let expires = copy.profileExpires {
                    LabeledContent("Profile Expires") {
                        Text(expires, format: .dateTime.day().month().year())
                            .foregroundStyle(expires < Date() ? Color.red : Color.primary)
                    }
                }
            }

            if let record {
                SigningRecordSections(record: record)
            } else {
                Section {
                    Text("This copy was signed before WarpSign kept a record of each sign. Sign the app again to see what each part was signed with, and the steps.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle(app.displayName)
        .navigationBarTitleDisplayMode(.inline)
    }

    private func signingReportSection(_ record: SigningRecord) -> some View {
        let report = SigningReport.markdown(app: app, copy: copy, record: record)
        return Section {
            Button("Copy Signing Report", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = report
            }
            ShareLink(item: report) {
                Label("Share Signing Report", systemImage: "doc.text")
            }
        } footer: {
            Text("Everything below as Markdown: options, each part's bundle IDs, capabilities and entitlements as a diff with reasons, profiles and the full log. For bug reports, or to ask an AI what changed.")
        }
    }
}

/// How a signed copy was made: warnings, entitlements per component,
/// options and the steps, as kept when it was signed
private struct SigningRecordSections: View {
    let record: SigningRecord

    var body: some View {
        if !record.warnings.isEmpty {
            Section("Warnings") {
                ForEach(record.warnings, id: \.self) { warning in
                    Label(warning, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                        .font(.callout)
                }
            }
        }

        let components = listedComponents(record.components)
        if !components.isEmpty {
            Section {
                ForEach(components) { component in
                    NavigationLink {
                        SignedComponentView(component: component)
                    } label: {
                        SignedComponentRow(component: component)
                    }
                }
            } header: {
                Text("Entitlements")
            } footer: {
                Text("What each part was signed with, and what changed. Frameworks are signed without entitlements.")
            }
        }

        if !record.profiles.isEmpty {
            Section("Profiles") {
                ForEach(record.profiles, id: \.bundlePath) { profile in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack {
                            Text(profile.bundlePath.isEmpty ? "App" : profile.bundlePath)
                            Spacer()
                            Badge(profile.ppqCheck ? "PPQ on" : "PPQ off", color: profile.ppqCheck ? .orange : .green)
                        }
                        Text(profile.name)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
        }

        Section {
            NavigationLink {
                SigningOptionsSummary(options: record.options, customIcon: record.customIcon)
            } label: {
                Label("Signing Options", systemImage: "slider.horizontal.3")
            }
        }

        if !record.steps.isEmpty {
            Section {
                SavedStepListView(steps: record.steps)
            } header: {
                Text("Steps")
            } footer: {
                CopyLogButton(savedSteps: record.steps)
            }
        }
    }
}

/// The options a copy was signed with
private struct SigningOptionsSummary: View {
    let options: SigningOptions?
    let customIcon: Bool

    var body: some View {
        List {
            if let options {
                Section {
                    LabeledContent("Certificate", value: options.certificate.title)
                }
                Section("Identifiers") {
                    row("Unique Identifiers", options.encodeIds)
                    row("Patch IDs in Binaries", options.patchIds)
                    row("Keep Original Bundle ID", options.forceOriginalId)
                }
                Section("App") {
                    LabeledContent("Name", value: options.bundleName.isEmpty ? "Unchanged" : options.bundleName)
                    LabeledContent("Icon", value: customIcon ? "Replaced" : "Unchanged")
                    row("Support All Devices", options.patchAllDevices)
                    row("File Sharing", options.patchFileSharing)
                    row("ProMotion (120 Hz)", options.patchPromotion)
                    row("Fullscreen on iPad", options.patchFullscreen)
                    row("All Orientations", options.patchOrientation)
                    row("Game Mode", options.patchGameMode)
                    row("Remove URL Schemes", options.removeUrlSchemes)
                    LabeledContent("Status Bar", value: options.statusBar.title)
                    LabeledContent("Appearance", value: options.interfaceStyle.title)
                }
                Section {
                    row("Allow Debugging", options.patchDebug)
                }
            } else {
                Text("The options weren't recorded.").foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Signing Options")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func row(_ title: String, _ on: Bool) -> some View {
        LabeledContent(title, value: on ? "On" : "Off")
    }
}
