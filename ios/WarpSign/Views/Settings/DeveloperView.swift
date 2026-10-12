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
        }
        .navigationTitle("Developer")
    }
}
