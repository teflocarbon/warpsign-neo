import SwiftUI

/// Switches for seeing flows an account isn't in right now. Off by default.
enum DeveloperOptions {
    private static let pretendNotGrantedKey = "developer.pretendNotGranted"

    /// Drop keys from capabilities Apple must grant (Family Controls
    /// (Distribution) and the like) when signing, with the usual warning,
    /// even if the team has them
    static var pretendNotGranted: Bool {
        get { UserDefaults.standard.bool(forKey: pretendNotGrantedKey) }
        set { UserDefaults.standard.set(newValue, forKey: pretendNotGrantedKey) }
    }
}

struct DeveloperView: View {
    @Environment(AccountStore.self) private var account
    @State private var pretendNotGranted = DeveloperOptions.pretendNotGranted

    var body: some View {
        @Bindable var account = account
        List {
            Section {
                Toggle("Pretend Capabilities Aren't Granted", isOn: $pretendNotGranted)
                    .onChange(of: pretendNotGranted) { _, value in
                        DeveloperOptions.pretendNotGranted = value
                    }
            } footer: {
                Text("Signing removes entitlements from capabilities Apple must grant, such as Family Controls (Distribution), with the warning a team without them gets, even though yours has them. Signed apps lose those entitlements while this is on.")
            }

            Section {
                Toggle("Show Certificate Migration", isOn: $account.forceMigration)
            } footer: {
                Text("Offers moving to a new certificate in the Library and Settings as if your certificates were about to expire. Nothing changes until you go through with it.")
            }

            AnisetteProbeSection()
        }
        .navigationTitle("Developer")
    }
}

/// Spike: docs/spikes/anisette-on-device.md. Shows whether iOS's AuthKit
/// hands this app anisette data, without ever showing the values.
private struct AnisetteProbeSection: View {
    @State private var findings: [AnisetteProbe.Finding] = []
    @State private var running = false

    var body: some View {
        Section {
            Button {
                running = true
                Task {
                    findings = await AnisetteProbe.run()
                    running = false
                }
            } label: {
                HStack {
                    Text("Probe Anisette from AuthKit")
                    if running {
                        Spacer()
                        ProgressView()
                    }
                }
            }
            .disabled(running)

            ForEach(findings) { finding in
                HStack(alignment: .firstTextBaseline) {
                    Image(systemName: finding.ok == nil ? "info.circle" : finding.ok! ? "checkmark.circle.fill" : "xmark.circle")
                        .foregroundStyle(finding.ok == nil ? Color.secondary : finding.ok! ? Color.green : Color.red)
                    VStack(alignment: .leading) {
                        Text(finding.step)
                        Text(finding.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
        } footer: {
            Text("Asks iOS for the device headers Apple's sign-in service expects, as Xcode does on a Mac, and lists which came back and their lengths. It never shows the values and sends nothing. Experimental: see docs/spikes/anisette-on-device.md.")
        }
    }
}
