import SwiftUI
import UniformTypeIdentifiers

struct AppsView: View {
    @Environment(AppModel.self) private var model
    @State private var showingImporter = false

    private static let ipaType = UTType("com.apple.itunes.ipa")
        ?? UTType(filenameExtension: "ipa")
        ?? .data

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Apps")
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
            List {
                Section("Reading \(fileName)") {
                    StepListView(steps: model.steps)
                }
            }
        case .inspected(let info):
            AppInfoView(info: info, steps: model.steps)
        case .failed(let message):
            List {
                Section {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                }
                Section {
                    StepListView(steps: model.steps)
                } header: {
                    Text("Steps")
                } footer: {
                    CopyLogButton(steps: model.steps)
                }
            }
        }
    }
}

struct AppInfoView: View {
    let info: AppInfo
    let steps: [StepState]
    @State private var showingSteps = false

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
                DisclosureGroup("Steps", isExpanded: $showingSteps) {
                    StepListView(steps: steps)
                }
            } footer: {
                HStack {
                    Text("warpsign-core \(coreVersion())")
                    Spacer()
                    CopyLogButton(steps: steps)
                }
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
                if component.isPrimary {
                    Badge("App ID", color: .accentColor)
                }
            }
            Text(component.bundleId)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
        }
    }
}

struct Badge: View {
    let text: String
    let color: Color

    init(_ text: String, color: Color) {
        self.text = text
        self.color = color
    }

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(color.opacity(0.15), in: .capsule)
    }
}
