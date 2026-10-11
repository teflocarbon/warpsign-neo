import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @State private var showingImporter = false

    private static let ipaType = UTType("com.apple.itunes.ipa")
        ?? UTType(filenameExtension: "ipa")
        ?? .data

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("WarpSign")
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        Button("Open IPA", systemImage: "plus") {
                            showingImporter = true
                        }
                    }
                }
                .fileImporter(
                    isPresented: $showingImporter,
                    allowedContentTypes: [Self.ipaType]
                ) { result in
                    if case .success(let url) = result {
                        Task { await model.inspect(url) }
                    }
                }
        }
    }

    @ViewBuilder
    private var content: some View {
        switch model.state {
        case .idle:
            ContentUnavailableView {
                Label("No App Selected", systemImage: "app.dashed")
            } description: {
                Text("Open an IPA to see what's inside it.")
            } actions: {
                Button("Open IPA") { showingImporter = true }
                    .buttonStyle(.borderedProminent)
            }
        case .inspecting(let fileName):
            ProgressView("Reading \(fileName)…")
        case .inspected(let info):
            AppInfoView(info: info)
        case .failed(let message):
            ContentUnavailableView(
                "Couldn't Read App",
                systemImage: "exclamationmark.triangle",
                description: Text(message)
            )
        }
    }
}

struct AppInfoView: View {
    let info: AppInfo

    var body: some View {
        List {
            Section("App") {
                LabeledContent("Name", value: info.displayName ?? "—")
                LabeledContent("Bundle ID", value: info.bundleId)
                LabeledContent("Version", value: info.version ?? "—")
            }
            Section("Signed For") {
                if info.teamIds.isEmpty {
                    Text("Not signed").foregroundStyle(.secondary)
                } else {
                    ForEach(info.teamIds, id: \.self) { Text($0).monospaced() }
                }
            }
            Section("Components") {
                ForEach(info.components, id: \.path) { component in
                    ComponentRow(component: component)
                }
            }
            Section {
            } footer: {
                Text("warpsign-core \(coreVersion())")
            }
        }
    }
}

struct ComponentRow: View {
    let component: Component

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(component.path.isEmpty ? "App" : component.path)
                    .font(.body)
                if component.isPrimary {
                    Text("App ID")
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.tint.opacity(0.15), in: .capsule)
                }
            }
            Text(component.bundleId)
                .font(.caption)
                .monospaced()
                .foregroundStyle(.secondary)
        }
    }
}
