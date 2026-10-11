import SwiftUI

/// The team's devices: signed apps install only on these
struct DevicesView: View {
    @Environment(AccountStore.self) private var account
    @State private var devices: [DeviceInfo]?
    @State private var error: String?

    var body: some View {
        List {
            if let error {
                Section { ErrorRow(message: error) }
            }
            if let devices {
                Section {
                    ForEach(devices, id: \.id) { device in
                        HStack(spacing: 12) {
                            Image(systemName: device.deviceClass == "IPAD" ? "ipad" : "iphone")
                                .font(.title2)
                                .foregroundStyle(.secondary)
                                .frame(width: 30)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(device.name)
                                if let model = device.model {
                                    Text(model).font(.caption).foregroundStyle(.secondary)
                                }
                                Text(device.udid)
                                    .font(.caption2.monospaced())
                                    .foregroundStyle(.tertiary)
                                    .textSelection(.enabled)
                            }
                        }
                    }
                } footer: {
                    Text("Signed apps install only on these devices. To add one, register its UDID at developer.apple.com, or with `warpsign device add` on a computer.")
                }
            }
        }
        .overlay {
            if devices == nil && error == nil {
                ProgressView()
            } else if devices?.isEmpty == true {
                ContentUnavailableView(
                    "No Devices",
                    systemImage: "iphone.slash",
                    description: Text("Register this iPhone's UDID at developer.apple.com before signing.")
                )
            }
        }
        .navigationTitle("Devices")
        .refreshable { await load() }
        .task { await load() }
    }

    private func load() async {
        do {
            devices = try await account.devices()
            error = nil
        } catch {
            self.error = error.userMessage
        }
    }
}
