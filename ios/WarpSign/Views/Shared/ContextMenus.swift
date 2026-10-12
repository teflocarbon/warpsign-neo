import SwiftUI
import UIKit

// Press-and-hold menus: the things you'd otherwise open a page for, and
// the details worth copying.

/// An app in the library
struct LibraryAppMenu: View {
    @Environment(LibraryStore.self) private var library
    @Environment(SourcesStore.self) private var sources
    let app: LibraryApp
    let open: () -> Void
    let link: () -> Void
    let remove: () -> Void
    private var updater: Updater { .shared }

    var body: some View {
        Button("Open", systemImage: "arrow.up.right.square", action: open)

        if let update = updater.available[app.id] {
            Section("\(update.version.version) Available") {
                Button("Update to \(update.version.version)", systemImage: "arrow.down.circle") {
                    Task { await updater.run([app.id], automatic: false) }
                }
                .disabled(updater.isUpdating)
                Button("Skip \(update.version.version)", systemImage: "forward") {
                    updater.skip(app.id)
                }
            }
        }

        Section {
            if app.sourceURL != nil {
                Toggle(isOn: Binding(
                    get: { app.updatesAutomatically },
                    set: { library.setAutoUpdate($0, for: app) }
                )) {
                    Label("Update Automatically", systemImage: "arrow.triangle.2.circlepath")
                }
                if let skipped = app.skippedVersion {
                    Button("Don't Skip \(skipped)", systemImage: "arrow.uturn.backward") {
                        updater.stopSkipping(app.id)
                    }
                }
                Button("Stop Following \(app.origin ?? "Source")", systemImage: "xmark.circle") {
                    library.unlink(app)
                    updater.evaluate()
                }
            } else if app.updateOrigin != .bundled {
                Button("Link to a Source…", systemImage: "link", action: link)
            }
        }

        Section {
            Button("Copy Bundle ID", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = app.bundleId
            }
            if let version = app.version {
                Button("Copy Version", systemImage: "number") {
                    UIPasteboard.general.string = app.build.map { $0 == version ? version : "\(version) (\($0))" } ?? version
                }
            }
            if let link = app.link {
                Button("Copy Download Link", systemImage: "link") {
                    UIPasteboard.general.string = link
                }
            }
            ShareLink(item: LibraryStore.ipaURL(of: app.id)) {
                Label("Share Original IPA", systemImage: "square.and.arrow.up")
            }
            if let newest = app.signedCopies.max(by: { $0.date < $1.date }) {
                ShareLink(item: library.signedURL(newest, of: app)) {
                    Label("Share Signed IPA", systemImage: "checkmark.seal")
                }
            }
        }

        Button("Remove from Library", systemImage: "trash", role: .destructive, action: remove)
    }
}

/// A download in the list
struct DownloadMenu: View {
    let item: DownloadManager.Item
    private var downloads: DownloadManager { .shared }

    var body: some View {
        switch item.state {
        case .downloading:
            if case .library = item.purpose {
                Button("Pause", systemImage: "pause") { downloads.pause(item.id) }
            }
            Button("Cancel", systemImage: "xmark", role: .destructive) { downloads.cancel(item.id) }
        case .stopped:
            Button("Resume", systemImage: "play") { downloads.resume(item.id) }
            Button("Cancel", systemImage: "xmark", role: .destructive) { downloads.cancel(item.id) }
        case .failed:
            if case .library = item.purpose {
                Button("Try Again", systemImage: "arrow.clockwise") { downloads.resume(item.id) }
            }
            Button("Remove", systemImage: "trash", role: .destructive) { downloads.remove(item.id) }
        case .importing:
            EmptyView()
        }
        Section {
            Button("Copy Link", systemImage: "link") {
                UIPasteboard.general.string = item.url.absoluteString
            }
            if case .failed(let message) = item.state {
                Button("Copy Error", systemImage: "exclamationmark.bubble") {
                    UIPasteboard.general.string = message
                }
            }
        }
    }
}

/// An app in a source
struct SourceAppMenu: View {
    @Environment(SourcesStore.self) private var sources
    let app: SourceApp
    let source: Source

    var body: some View {
        if let latest = app.versions.first {
            Button("Get \(latest.version)", systemImage: "arrow.down.circle") {
                sources.download(latest, of: app, from: source)
            }
            .disabled(DownloadManager.shared.items.contains { $0.url.absoluteString == latest.downloadUrl })
        }
        Section {
            Button("Copy Bundle ID", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = app.bundleId
            }
            if let latest = app.versions.first {
                Button("Copy Download Link", systemImage: "link") {
                    UIPasteboard.general.string = latest.downloadUrl
                }
                if let url = URL(string: latest.downloadUrl) {
                    ShareLink(item: url) {
                        Label("Share Download Link", systemImage: "square.and.arrow.up")
                    }
                }
            }
        }
    }
}

/// A version in a source's version history
struct SourceVersionMenu: View {
    let version: AppVersion

    var body: some View {
        Button("Copy Version", systemImage: "number") {
            UIPasteboard.general.string = version.build.map { $0 == version.version ? version.version : "\(version.version) (\($0))" }
                ?? version.version
        }
        Button("Copy Download Link", systemImage: "link") {
            UIPasteboard.general.string = version.downloadUrl
        }
        if let notes = version.notes {
            Button("Copy Release Notes", systemImage: "doc.plaintext") {
                UIPasteboard.general.string = notes
            }
        }
    }
}

/// A row of details: copy what it says
struct CopyMenu: View {
    let title: String
    let value: String

    var body: some View {
        Button("Copy \(title)", systemImage: "doc.on.doc") {
            UIPasteboard.general.string = value
        }
    }
}
