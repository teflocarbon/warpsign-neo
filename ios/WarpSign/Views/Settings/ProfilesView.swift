import SwiftUI
import UniformTypeIdentifiers

/// Diagnostic view of provisioning profiles from your own account: whether
/// PPQ checking is on, and whether a profile is usable at all
struct ProfilesView: View {
    @Environment(InspectionModel.self) private var model
    @State private var showingImporter = false

    private static let profileType = UTType("com.apple.mobileprovision")
        ?? UTType(filenameExtension: "mobileprovision")
        ?? .data

    var body: some View {
        List {
            if let error = model.profileError {
                Section {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
            if model.profiles.isEmpty {
                ContentUnavailableView {
                    Label("No Profiles", systemImage: "doc.badge.gearshape")
                } description: {
                    Text("Add a provisioning profile from your developer account to check it for PPQ.")
                }
            } else {
                Section {
                    ForEach(model.profiles, id: \.self) { profile in
                        ProfileRow(profile: profile)
                    }
                    .onDelete { model.removeProfiles(at: $0) }
                } footer: {
                    Text("Teams created after 6 June 2021 get PPQ checking: apps check in with Apple on first launch, so WarpSign won't keep original bundle IDs with them.")
                }
            }
        }
        .navigationTitle("Profiles")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Add Profile", systemImage: "plus") {
                    showingImporter = true
                }
            }
        }
        .fileImporter(
            isPresented: $showingImporter,
            allowedContentTypes: [Self.profileType]
        ) { result in
            if case .success(let url) = result {
                Task { await model.addProfile(url) }
            }
        }
    }
}

private struct ProfileRow: View {
    let profile: ProfileInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(profile.name ?? "Unnamed profile")
                    .font(.headline)
                Spacer()
                if profile.ppqCheck {
                    Badge("PPQ on", color: .orange)
                } else {
                    Badge("PPQ off", color: .green)
                }
            }
            if profile.isWildcard {
                Label("Wildcard profiles aren't supported", systemImage: "nosign")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            Group {
                if !profile.teamIds.isEmpty {
                    Text("Team \(profile.teamIds.joined(separator: ", "))")
                }
                Text("\(profile.deviceCount) device\(profile.deviceCount == 1 ? "" : "s")")
                if let expiresAt = profile.expiresAt {
                    Text("Expires \(Date(timeIntervalSince1970: TimeInterval(expiresAt)), style: .date)")
                }
                if let uuid = profile.uuid {
                    Text(uuid).monospaced()
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 2)
    }
}
