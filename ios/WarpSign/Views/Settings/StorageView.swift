import SwiftUI

/// What WarpSign keeps on this device, and cleaning it up
struct StorageView: View {
    @Environment(LibraryStore.self) private var library
    @State private var usage: StorageUsage?
    @State private var confirming: Cleanup?

    private enum Cleanup: Identifiable {
        case superseded, caches, temporary
        var id: Self { self }
    }

    var body: some View {
        List {
            Section {
                row("Imported Apps", \.originals, systemImage: "square.and.arrow.down")
                row("Signed Copies", \.signedCopies, systemImage: "checkmark.seal")
                row("Caches", \.caches, systemImage: "archivebox")
                row("Temporary Files", \.temporary, systemImage: "clock.arrow.circlepath")
                LabeledContent {
                    size(\.total).fontWeight(.semibold)
                } label: {
                    Text("Total").fontWeight(.semibold)
                }
            } header: {
                Text("Usage")
            } footer: {
                Text("Imported apps are the IPAs you added; signed copies are kept so they can be installed or shared again. Caches hold provisioning profiles and capabilities, so signing an app again needs fewer requests to Apple.")
            }

            Section {
                Button(role: .destructive) {
                    confirming = .superseded
                } label: {
                    LabeledContent {
                        if let usage, usage.supersededCount > 0 {
                            Text("\(usage.supersededCount) · \(ByteCountFormatter.file(usage.superseded))")
                        }
                    } label: {
                        Label("Remove Superseded Copies", systemImage: "arrow.3.trianglepath")
                    }
                }
                .disabled((usage?.supersededCount ?? 0) == 0)
                Button(role: .destructive) {
                    confirming = .caches
                } label: {
                    Label("Clear Caches", systemImage: "archivebox")
                }
                Button(role: .destructive) {
                    confirming = .temporary
                } label: {
                    Label("Clear Temporary Files", systemImage: "trash")
                }
                .disabled(KeepAlive.shared.isHolding)
            } header: {
                Text("Clean Up")
            } footer: {
                Text("Superseded copies are older signed copies of an app with a newer one from the same certificate. Temporary files are what signing, inspecting and downloads left behind; anything from the last 15 minutes stays.")
            }
        }
        .navigationTitle("Storage")
        .task { await refresh() }
        .refreshable { await refresh() }
        .confirmationDialog(
            title(confirming),
            isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
            titleVisibility: .visible,
            presenting: confirming
        ) { cleanup in
            Button(buttonTitle(cleanup), role: .destructive) {
                Task { await clean(cleanup) }
            }
        } message: { cleanup in
            Text(message(cleanup))
        }
    }

    private func row(_ title: String, _ value: KeyPath<StorageUsage, UInt64>, systemImage: String) -> some View {
        LabeledContent {
            size(value)
        } label: {
            Label(title, systemImage: systemImage)
        }
    }

    @ViewBuilder
    private func size(_ value: KeyPath<StorageUsage, UInt64>) -> some View {
        if let usage {
            Text(ByteCountFormatter.file(usage[keyPath: value]))
        } else {
            ProgressView()
        }
    }

    private func title(_ cleanup: Cleanup?) -> String {
        switch cleanup {
        case .superseded: "Remove superseded copies?"
        case .caches: "Clear caches?"
        case .temporary: "Clear temporary files?"
        case nil: ""
        }
    }

    private func buttonTitle(_ cleanup: Cleanup) -> String {
        switch cleanup {
        case .superseded: "Remove \(usage?.supersededCount ?? 0) Copies"
        case .caches: "Clear Caches"
        case .temporary: "Clear Temporary Files"
        }
    }

    private func message(_ cleanup: Cleanup) -> String {
        switch cleanup {
        case .superseded:
            "The newest signed copy of each app, for each certificate, stays. Installed apps aren't affected."
        case .caches:
            "The next sign of each app downloads its provisioning profiles again. Nothing changes on the portal."
        case .temporary:
            "Only files left behind by finished work go."
        }
    }

    private func clean(_ cleanup: Cleanup) async {
        switch cleanup {
        case .superseded:
            library.removeSupersededCopies()
        case .caches:
            await StorageUsage.run { StorageUsage.clearCaches() }
        case .temporary:
            await StorageUsage.run { StorageUsage.clearTemporary() }
        }
        await refresh()
    }

    private func refresh() async {
        let files = library.storageFiles()
        usage = await StorageUsage.run { StorageUsage.measure(files) }
    }
}
