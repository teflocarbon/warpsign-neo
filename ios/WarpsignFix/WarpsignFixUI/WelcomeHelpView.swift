import SwiftUI

struct WelcomeHelpView: View {
    var body: some View {
        List {
            // Banner about Warpsign
            HStack {
                Image(systemName: "info.circle.fill") // Changed icon to info
                    .foregroundColor(.white)
                Spacer() // Push icon to left
                Text("WarpsignFix works best when signing with Warpsign.")
                    .foregroundColor(.white)
                    .font(.footnote)
                Spacer() // Center text
            }
            .padding(10) // Add padding inside the banner
            .background(Color.purple)
            .cornerRadius(8)
            // Remove list styling for this row
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)

            Section(header: Text("Welcome")) {
                Text("Thank you for using WarpsignFix!\n\nIt's highly recommended to review and adjust the settings after installation. The default settings prioritize stability but might disable or alter functionality needed by the app.")
            }

            Section(header: Text("About WarpsignFix")) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("WarpsignFix is designed to be a comprehensive, swiss-army knife of sideload fixes.")
                        .padding(.bottom, 5)
                    Text("All options within it are completely customizable, allowing it to work over a huge array of applications, solving many common sideloading problems.")
                        .font(.caption)
                        .foregroundColor(.gray)
                }
            }

            Section(header: Text("Accessing Settings")) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("There are several ways to open WarpsignFix settings:")
                        .padding(.bottom, 5)

                    HStack {
                        Image(systemName: "app.badge")
                            .foregroundColor(.blue)
                            .frame(width: 25)
                        Text("**Home Screen Shortcut:** Long-press the app icon and choose 'WarpsignFix Settings'.")
                    }
                    Text("Note: This shortcut will only work if the app is open in the background.")
                        .font(.caption)
                        .foregroundColor(.gray)
                        .padding(.leading, 30) // Indent note

                    Divider().padding(.vertical, 5)

                    HStack {
                        Image(systemName: "airplane")
                            .foregroundColor(.orange)
                            .frame(width: 25)
                        Text("**Airplane Mode:** Activate Airplane Mode, then tap the orange dot that appears in the top-left corner of the app.")
                    }
                     Text("The dot disappears after a few seconds or when tapped.")
                        .font(.caption)
                        .foregroundColor(.gray)
                        .padding(.leading, 30) // Indent note
                }
            }

            Section(header: Text("Troubleshooting & Crash Recovery")) {
                 VStack(alignment: .leading, spacing: 10) {
                    // Introductory text
                    Text("Some settings may cause the app to crash, this can be for a variety of reasons. WarpsignFix has several possible methods to rescue from a crash loop:")
                        .padding(.bottom, 5)

                    // Method 1: Reinstalling
                    HStack(alignment: .top) { // Align to top for multi-line text
                         Image(systemName: "trash") // Icon for reinstalling
                            .foregroundColor(.red)
                            .frame(width: 25)
                         VStack(alignment: .leading) {
                            Text("**Reinstall App:** Delete and reinstall the application.")
                            Text("This is the simplest method and resets all WarpsignFix settings to their safe defaults.")
                                .font(.caption)
                                .foregroundColor(.gray)
                         }
                    }

                    Divider().padding(.vertical, 5)

                    // Method 2: External Configuration File
                    HStack(alignment: .top) { // Align to top
                         Image(systemName: "doc.text.fill") // Use filled icon
                            .foregroundColor(.gray)
                            .frame(width: 25)
                         VStack(alignment: .leading) {
                            Text("**External Config File:** Manually edit the `warpsignfix_config.plist` file.")
                            Text("This file is automatically created in the app's 'Documents' directory. Instructions are in `warpsignfix_config_README.txt` (in the same directory).")
                                .font(.caption)
                                .foregroundColor(.gray)
                            Text("If you cannot properly edit the file, deleting it will reset WarpsignFix to safe defaults as well.")
                                .font(.caption)
                                .foregroundColor(.gray)
                            Text("Access via Files app requires enabling 'Supports Document Browser' (`UISupportsDocumentBrowser`) in entitlements during signing.")
                                .font(.caption)
                                .foregroundColor(.gray)
                                .padding(.top, 1)
                         }
                    }
                 }
            }

            Section(header: Text("Accessing Logs on macOS")) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("If you are using macOS, you can connect your device and use the 'Console' app to view logs.")
                        .padding(.bottom, 5)
                    Text("1. Connect your device to your Mac.")
                    Text("2. Open the 'Console' app (found in Applications > Utilities).")
                    Text("3. In the top left, filter the console for 'WarpsignFix'.")
                    Text("This will provide comprehensive logs for everything that WarpsignFix is doing.")
                        .font(.caption)
                        .foregroundColor(.gray)
                }
            }

        }
        .listStyle(InsetGroupedListStyle())
        .navigationTitle("Welcome & Help")
        .navigationBarTitleDisplayMode(.inline)
    }
}

// Add a preview for development if needed
#if DEBUG
struct WelcomeHelpView_Previews: PreviewProvider {
    static var previews: some View {
        NavigationView {
            WelcomeHelpView()
        }
    }
}
#endif 