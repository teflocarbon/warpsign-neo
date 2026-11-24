
import SwiftUI

struct LearnMoreView: View {
    // Define the symptoms text here for organization
    private let keychainSymptoms = [
        "App is not able to stay logged in.",
        "App refuses to let the user log in.",
        "App may say you aren't logged in if you try to share something to it.",
        "App logs in correctly, but exhibits odd behaviour like broken notifications."
    ]

    private let appGroupSymptoms = [
        "App doesn't show up in widgets selection.",
        "Dynamic Island is completely broken.",
        "Control Centre media player is unable to control the app.",
        "Lockscreen media player is unable to control the app."
    ]

    private let cloudKitSymptoms = [
        "App instantly crashes upon launch or specific actions."
    ]

    var body: some View {
        List {
            // Section for Keychain Symptoms
            Section(header: Text("Common Keychain Issues")) {
                Text("If an app has keychain problems, you might experience:")
                    .font(.subheadline) // Slightly smaller intro text
                    .padding(.bottom, 2)

                ForEach(keychainSymptoms, id: \.self) { symptom in
                    Text("• \(symptom)") // Use bullet points for clarity
                }
            }

            // Section for App Group Symptoms
            Section(header: Text("Common App Group Issues")) {
                 Text("If an app has App Group problems, you might experience:")
                    .font(.subheadline)
                    .padding(.bottom, 2)

                ForEach(appGroupSymptoms, id: \.self) { symptom in
                    Text("• \(symptom)")
                }
            }

            // Section for CloudKit Symptoms
            Section(header: Text("Common CloudKit Issues")) {
                 Text("If an app has CloudKit problems, you might experience:")
                    .font(.subheadline)
                    .padding(.bottom, 2)

                ForEach(cloudKitSymptoms, id: \.self) { symptom in
                    Text("• \(symptom)")
                }
            }
        }
        .listStyle(InsetGroupedListStyle()) // Consistent styling
        .navigationTitle("Common Issues") // Title for this view
        .navigationBarTitleDisplayMode(.inline) // Keep title inline
    }
}
