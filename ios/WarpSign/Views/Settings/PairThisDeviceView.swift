import SwiftUI
import UIKit

/// Making a pairing file here, without a computer (iOS 27+): WarpSign
/// waits on the local network, the user taps "Pair with WarpSign" in
/// Settings and types the code shown here
struct PairThisDeviceView: View {
    @Environment(DeviceStore.self) private var device
    @Environment(\.dismiss) private var dismiss
    @State private var pairer = OnDevicePairer()

    var body: some View {
        NavigationStack {
            List {
                switch pairer.phase {
                case .idle:
                    Section {
                        Button("Start Pairing", systemImage: "link") {
                            pairer.start(device: device)
                        }
                    } footer: {
                        Text("WarpSign shows itself to this device as a computer it can pair with, on your Wi-Fi network. Nothing leaves the device. The pairing file it makes lets WarpSign install apps here, as one made with `warpsign pair` does.")
                    }
                case .waiting:
                    Section {
                        HStack(spacing: 12) {
                            ProgressView()
                            Text("Waiting for Settings…").foregroundStyle(.secondary)
                        }
                    }
                    steps
                case .code(let code):
                    Section {
                        VStack(spacing: 12) {
                            Text("Enter this code in Settings")
                                .font(.headline)
                            Text(code)
                                .font(.system(size: 44, weight: .bold, design: .rounded))
                                .monospacedDigit()
                                .tracking(6)
                                .textSelection(.enabled)
                            ProgressView()
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                    } footer: {
                        Text("It's in a notification too, if Settings is in front.")
                    }
                case .paired(let name):
                    Section {
                        Label("Paired with \(name)", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                    } footer: {
                        Text("The pairing file is saved. WarpSign installs through its loopback VPN from now on.")
                    }
                case .failed(let message):
                    Section {
                        ErrorRow(message: message)
                        Button("Try Again", systemImage: "arrow.clockwise") {
                            pairer.start(device: device)
                        }
                    }
                }
            }
            .navigationTitle("Pair This Device")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .onDisappear { pairer.cancel() }
    }

    private var steps: some View {
        Section {
            Label("Open Settings → Privacy & Security → Developer Mode.", systemImage: "1.circle")
            Label("Tap Pair with \(pairer.hostName).", systemImage: "2.circle")
            Label("Enter the code WarpSign shows.", systemImage: "3.circle")
            Button("Open Settings", systemImage: "gear") {
                if let url = URL(string: UIApplication.openSettingsURLString) {
                    UIApplication.shared.open(url)
                }
            }
        } header: {
            Text("In Settings")
        } footer: {
            Text("Developer Mode must be on, and this device on Wi-Fi. WarpSign keeps waiting while you're in Settings.")
        }
    }
}
