import SwiftUI

struct SettingsView: View {
    @Environment(AccountStore.self) private var account
    @Environment(DeviceStore.self) private var device
    @State private var signingIn = false
    @State private var confirmingSignOut = false

    var body: some View {
        @Bindable var account = account
        NavigationStack {
            List {
                Section {
                    switch account.phase {
                    case .restoring:
                        HStack(spacing: 12) {
                            ProgressView()
                            Text("Checking your session…").foregroundStyle(.secondary)
                        }
                    case .signedOut:
                        if let problem = account.sessionProblem {
                            ErrorRow(message: problem)
                        }
                        Button("Sign In with Apple ID…", systemImage: "person.crop.circle.badge.plus") {
                            signingIn = true
                        }
                    case .signedIn:
                        LabeledContent("Apple ID", value: account.appleId ?? "—")
                        if account.teams.count > 1 {
                            Picker("Team", selection: $account.selectedTeamId) {
                                ForEach(account.teams, id: \.teamId) { team in
                                    Text(team.name).tag(Optional(team.teamId))
                                }
                            }
                        } else if let team = account.selectedTeam {
                            LabeledContent("Team", value: team.name)
                        }
                        if let team = account.selectedTeam {
                            LabeledContent("Team ID") {
                                Text(team.teamId).monospaced()
                            }
                            if let ppq = account.teamUsesPPQ(team.teamId) {
                                LabeledContent("PPQ Checking") {
                                    Badge(ppq ? "On" : "Off", color: ppq ? .orange : .green)
                                }
                            }
                        }
                        Button("Sign Out", role: .destructive) { confirmingSignOut = true }
                    }
                } header: {
                    Text("Apple Developer Account")
                } footer: {
                    Text("For paid Apple Developer Program members. Free accounts aren't supported.")
                }

                MigrationBanner()

                Section {
                    NavigationLink {
                        IdentityView()
                    } label: {
                        IdentitySummary()
                    }
                    ForEach([CertificateType.development, .distribution], id: \.self) { kind in
                        if account.identities[kind] != nil && account.isSignedIn {
                            NavigationLink {
                                MigrationView(kind: kind)
                            } label: {
                                Label(
                                    kind == .development ? "New Development Certificate" : "New Distribution Certificate",
                                    systemImage: "arrow.triangle.2.circlepath"
                                )
                            }
                        }
                    }
                } header: {
                    Text("Signing Identity")
                } footer: {
                    if account.needsBackup {
                        Text("Back up your signing identity, so a new device or reinstall doesn't use up another certificate.")
                            .foregroundStyle(.orange)
                    }
                }

                Section {
                    NavigationLink {
                        InstallingView()
                    } label: {
                        HStack {
                            Label("Installing", systemImage: "arrow.down.app")
                            Spacer()
                            if device.isReady {
                                Badge(device.method == .web ? "Over the Web" : "Pairing File", color: .green)
                            } else {
                                Badge("Not Set Up", color: .secondary)
                            }
                        }
                    }
                } footer: {
                    Text("Install signed apps straight onto this device.")
                }

                Section {
                    NavigationLink {
                        UpdatesSettingsView()
                    } label: {
                        Label("Automatic Updates", systemImage: "arrow.down.app")
                    }
                    NavigationLink {
                        StorageView()
                    } label: {
                        Label("Storage", systemImage: "internaldrive")
                    }
                }

                Section("Diagnostics") {
                    if account.isSignedIn {
                        NavigationLink {
                            DevicesView()
                        } label: {
                            Label("Registered Devices", systemImage: "iphone")
                        }
                    }
                    NavigationLink {
                        ProfilesView()
                    } label: {
                        Label("Check Provisioning Profiles", systemImage: "doc.badge.gearshape")
                    }
                    NavigationLink {
                        NetworkLogView()
                    } label: {
                        Label("Network Log", systemImage: "network")
                    }
                    NavigationLink {
                        DeveloperView()
                    } label: {
                        Label("Developer", systemImage: "hammer")
                    }
                }

                Section {
                    LabeledContent("WarpSign", value: AppInfo.versionText)
                    if let commit = AppInfo.commit {
                        LabeledContent("Commit", value: commit)
                    }
                    LabeledContent("warpsign-core", value: coreVersion())
                } header: {
                    Text("About")
                } footer: {
                    Text("WarpSign signs apps with your own Apple Developer account: it registers what each app needs and makes its profiles. It doesn't support free accounts, wildcard profiles, or signing with someone else's certificate.")
                }
            }
            .navigationTitle("Settings")
            .sheet(isPresented: $signingIn) {
                SignInView()
            }
            .confirmationDialog("Sign out?", isPresented: $confirmingSignOut, titleVisibility: .visible) {
                Button("Sign Out", role: .destructive) { account.signOut() }
            } message: {
                Text("Your signing identity stays on this device.")
            }
        }
    }
}

private struct IdentitySummary: View {
    @Environment(AccountStore.self) private var account

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("Certificates", systemImage: "person.badge.key")
            HStack(spacing: 6) {
                status(.development, "Development")
                status(.distribution, "Distribution")
            }
        }
    }

    private func status(_ kind: CertificateType, _ title: String) -> some View {
        let present = account.identities[kind] != nil
        return Badge(present ? title : "No \(title)", color: present ? .green : .secondary)
    }
}
