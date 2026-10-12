import SwiftUI

/// Settings → WarpSign Companion: the second app that installs WarpSign's
/// own updates. Set up from the copy bundled in WarpSign, then kept up to
/// date from it: by itself with a pairing file, or with a notification
/// to install it over the web.
struct CompanionView: View {
    @Environment(LibraryStore.self) private var library
    @Environment(DeviceStore.self) private var device
    @State private var signing: LibraryApp?
    @State private var settingUp = false
    private var updater: Updater { .shared }

    var body: some View {
        List {
            Section {
                Label {
                    Text("WarpSign can't install over itself, so Companion installs WarpSign's updates: WarpSign signs the update and hands it over, and Companion installs it.")
                } icon: {
                    Image(systemName: "arrow.triangle.2.circlepath.circle.fill").foregroundStyle(.tint)
                }
                .font(.callout)
            }

            if BundledCompanion.url == nil {
                Section {
                    Text("This copy of WarpSign doesn't include Companion. Builds from GitHub do.")
                        .foregroundStyle(.secondary)
                }
            } else if let companion = updater.companion {
                statusSection(companion)
                actionsSection(companion)
            } else {
                Section {
                    Button {
                        setUp()
                    } label: {
                        HStack {
                            Label("Set Up Companion", systemImage: "plus.app")
                            Spacer()
                            if settingUp { ProgressView() }
                        }
                    }
                    .disabled(settingUp)
                } footer: {
                    Text("Signs and installs the Companion that comes with this WarpSign (\(updater.bundledCompanionVersion ?? "bundled")). Once, then it keeps itself up to date.")
                }
            }
        }
        .navigationTitle("WarpSign Companion")
        .navigationDestination(item: $signing) { app in
            SignOptionsView(app: app)
        }
        .task { await device.refreshInstalledIfDue() }
    }

    private func statusSection(_ companion: LibraryApp) -> some View {
        Section {
            LabeledContent("Installed") {
                if let installed = device.installedApp(companion) {
                    Text(InstalledSection.versionText(installed.version, installed.build))
                } else if device.installed != nil {
                    Text(companion.signedCopies.isEmpty ? "Not yet" : "Not installed").foregroundStyle(.orange)
                } else {
                    Text(companion.signedCopies.isEmpty ? "Not yet" : "—").foregroundStyle(.secondary)
                }
            }
            LabeledContent("In This WarpSign", value: updater.bundledCompanionVersion ?? companion.versionText)
            if let update = updater.available[companion.id] {
                Label("\(update.version.version)\(update.version.build.map { " (\($0))" } ?? "") is ready to install", systemImage: "arrow.down.circle.fill")
                    .foregroundStyle(.tint)
            }
            UpdateStatusText(status: updater.status[companion.id])
        } header: {
            Text("Status")
        }
    }

    @ViewBuilder
    private func actionsSection(_ companion: LibraryApp) -> some View {
        Section {
            if updater.available[companion.id] != nil {
                Button("Update Now", systemImage: "arrow.down.circle") {
                    Task { await updater.run([companion.id], automatic: false) }
                }
                .disabled(updater.isUpdating)
            } else if companion.signedCopies.isEmpty {
                Button("Sign and Install…", systemImage: "signature") { signing = companion }
            } else if device.isMissing(companion), let newest = companion.signedCopies.max(by: { $0.date < $1.date }) {
                InstallButton(
                    ipa: library.signedURL(newest, of: companion),
                    name: companion.displayName,
                    icon: LibraryStore.folder(of: companion.id).appendingPathComponent("icon.png"),
                    bundleId: newest.bundleId
                )
            }
            Toggle("Update Automatically", isOn: Binding(
                get: { companion.updatesAutomatically },
                set: { library.setAutoUpdate($0, for: companion) }
            ))
            NavigationLink {
                AppDetailView(appId: companion.id)
            } label: {
                Label("Signed Copies and Details", systemImage: "info.circle")
            }
        } footer: {
            Text(device.method == .pairing
                 ? "When an update to WarpSign brings a newer Companion, it's signed and installed by itself, overnight while charging."
                 : "Installing over the web, iOS asks before every install: when an update to WarpSign brings a newer Companion, it's signed and WarpSign lets you know it's ready.")
        }
    }

    /// Add the bundled Companion and go to signing it
    private func setUp() {
        guard let url = BundledCompanion.url else { return }
        settingUp = true
        Task {
            defer { settingUp = false }
            if let app = await library.importFile(url, origin: BundledCompanion.origin) {
                signing = app
            }
        }
    }
}
