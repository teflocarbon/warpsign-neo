import SwiftUI

/// What's inside an IPA: its parts, their bundle IDs and entitlements, and
/// the team it's signed for, with the steps that found out
struct InspectionView: View {
    @Environment(InspectionModel.self) private var model
    @Environment(AccountStore.self) private var account
    let ipa: URL

    var body: some View {
        content
            .navigationTitle("Components")
            .navigationBarTitleDisplayMode(.inline)
            .task(id: ipa) { await model.inspect(ipa) }
            // Names come from the portal, so fetch them alongside
            .task { model.entitlementNames = await account.entitlementNames() }
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
            AppInfoView(info: info, steps: model.steps, names: model.entitlementNames)
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
    var names = EntitlementNames()
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
                    NavigationLink {
                        ComponentEntitlementsView(component: component, names: names)
                    } label: {
                        ComponentRow(component: component, names: names)
                    }
                }
            } header: {
                Text("Components")
            } footer: {
                Text("Each app and extension gets its own App ID and provisioning profile. Frameworks are re-signed. Tap one to see its entitlements.")
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
    let names: EntitlementNames
    private let entries: [PlistEntry]?

    init(component: Component, names: EntitlementNames = EntitlementNames()) {
        self.component = component
        self.names = names
        entries = Entitlements.parse(component.entitlementsXml)
    }

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
            Text(summary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
        }
    }

    /// "3 entitlements: Push Notifications, App Groups, …"
    private var summary: String {
        guard let entries else { return "Entitlements unreadable" }
        if entries.isEmpty { return "No entitlements" }
        let count = entries.count == 1 ? "1 entitlement" : "\(entries.count) entitlements"
        var seen = Set<String>()
        let named = entries.compactMap { names[$0.key]?.name }.filter { seen.insert($0).inserted }
        return named.isEmpty ? count : "\(count): \(named.joined(separator: ", "))"
    }
}
