import SwiftUI

/// What's inside an IPA: its parts, their bundle IDs and the team it's
/// signed for, with the steps that found out
struct InspectionView: View {
    @Environment(InspectionModel.self) private var model
    let ipa: URL

    var body: some View {
        content
            .navigationTitle("Components")
            .navigationBarTitleDisplayMode(.inline)
            .task(id: ipa) { await model.inspect(ipa) }
    }

    @ViewBuilder
    private var content: some View {
        switch model.state {
        case .idle, .inspecting:
            List {
                Section("Reading the app") {
                    StepListView(steps: model.steps)
                }
            }
        case .inspected(let info):
            AppInfoView(info: info, steps: model.steps)
        case .failed(let message):
            List {
                Section { ErrorRow(message: message) }
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
            Section("Signed For") {
                if info.teamIds.isEmpty {
                    Text("Not signed").foregroundStyle(.secondary)
                } else {
                    ForEach(info.teamIds, id: \.self) { Text($0).monospaced() }
                }
            }
            Section {
                ForEach(info.components, id: \.path) { component in
                    ComponentRow(component: component)
                }
            } header: {
                Text("Components")
            } footer: {
                Text("Each app and extension gets its own App ID and provisioning profile. Frameworks are re-signed.")
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
