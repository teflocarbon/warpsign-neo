import SwiftUI

/// Quick filters across the top of the library
enum LibraryFilter: String, CaseIterable, Identifiable {
    case all, updates, installed, notInstalled, automatic, manual

    var id: Self { self }

    var title: String {
        switch self {
        case .all: "All"
        case .updates: "Updates"
        case .installed: "Installed"
        case .notInstalled: "Not Installed"
        case .automatic: "Updating Automatically"
        case .manual: "Won't Update"
        }
    }

    var symbol: String {
        switch self {
        case .all: "square.stack.3d.up"
        case .updates: "arrow.down.circle"
        case .installed: "iphone"
        case .notInstalled: "iphone.slash"
        case .automatic: "arrow.triangle.2.circlepath"
        case .manual: "pause.circle"
        }
    }

    /// Only shown when it can be answered: installed needs the device asked
    @MainActor
    func isAvailable(device: DeviceStore) -> Bool {
        switch self {
        case .installed, .notInstalled: device.installed != nil
        default: true
        }
    }

    @MainActor
    func includes(_ app: LibraryApp, device: DeviceStore) -> Bool {
        let updater = Updater.shared
        switch self {
        case .all:
            return true
        case .updates:
            return updater.available[app.id] != nil
        case .installed:
            return device.installedApp(app) != nil
        case .notInstalled:
            return device.installed != nil && device.installedApp(app) == nil
        case .automatic:
            // Follows a source, and nothing stops it updating by itself
            return app.sourceURL != nil && updater.manualReason(app) == nil
        case .manual:
            // Imported, from a source it no longer follows, or held back
            return app.sourceURL == nil || updater.manualReason(app) != nil
        }
    }
}

/// The filters as chips, each with how many apps it has
struct LibraryFilterBar: View {
    @Environment(DeviceStore.self) private var device
    @Binding var selection: LibraryFilter
    let apps: [LibraryApp]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(LibraryFilter.allCases.filter { $0.isAvailable(device: device) }) { filter in
                    let count = filter == .all ? apps.count : apps.filter { filter.includes($0, device: device) }.count
                    let selected = filter == selection
                    Button {
                        withAnimation(.snappy) { selection = filter }
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: filter.symbol)
                            Text(filter.title)
                            Text("\(count)")
                                .foregroundStyle(selected ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
                        }
                        .font(.subheadline.weight(.medium))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 7)
                        .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
                        .background(
                            selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.fill.tertiary),
                            in: Capsule()
                        )
                    }
                    .buttonStyle(.plain)
                    // An empty filter is still choosable, just quieter
                    .opacity(count == 0 && !selected ? 0.5 : 1)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 4)
        }
    }
}
