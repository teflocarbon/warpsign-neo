import SwiftUI

@objc public class WSFSwiftUISettingsHostViewController: NSObject {
    @objc public static func createHostingController(startInHelpView: Bool = false) -> UIViewController {
        let settingsView = WSFSwiftUISettings(startInHelpView: startInHelpView)
        let hostingController = UIHostingController(rootView: settingsView)
        hostingController.preferredContentSize = CGSize(width: 350, height: 550)  // Increased height to accommodate new section
        hostingController.modalPresentationStyle = .formSheet
        return hostingController
    }
}
struct SettingsRowLabel: View {
    let iconName: String
    let iconColor: Color
    let text: String

    var body: some View {
        HStack(spacing: 15) {
            // Icon View with colored background
            Image(systemName: iconName)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 18, height: 18)
                .padding(6)
                .background(iconColor)
                .foregroundColor(.white)
                .cornerRadius(7)
                .frame(width: 30, height: 30, alignment: .center)

            // Text Label
            Text(text)
        }
    }
}

struct WSFSwiftUISettings: View {
    @ObservedObject private var configStore = WSFConfigStore.shared
    @State private var hasChangedStrategy = false
    
    let startInHelpView: Bool
    
    @State private var isHelpViewActive: Bool = false
    @State private var didPerformInitialHelpNavigation: Bool = false

    var body: some View {
        NavigationView {
            List {
                // Navigation links using the custom label
                NavigationLink(destination: KeychainSettingsView(configStore: configStore, hasChangedStrategy: $hasChangedStrategy)) {
                    SettingsRowLabel(iconName: "key.fill", iconColor: .orange, text: "Keychain")
                }

                NavigationLink(destination: CloudKitSettingsView(configStore: configStore, hasChangedStrategy: $hasChangedStrategy)) {
                    SettingsRowLabel(iconName: "cloud.fill", iconColor: .blue, text: "CloudKit")
                }

                NavigationLink(destination: AppGroupSettingsView(configStore: configStore, hasChangedStrategy: $hasChangedStrategy)) {
                    SettingsRowLabel(iconName: "folder.fill.badge.person.crop", iconColor: .green, text: "App Groups")
                }

                NavigationLink(destination: DiagnosticsSettingsView(configStore: configStore)) {
                    SettingsRowLabel(iconName: "stethoscope", iconColor: .red, text: "Diagnostics")
                }
                
                // Add Sideloading Symptoms link
                NavigationLink(destination: LearnMoreView()) {
                     SettingsRowLabel(iconName: "list.bullet.clipboard.fill", iconColor: .gray, text: "Sideloading Symptoms")
                 }

                // Add Welcome & Help link - NOW WITH isActive BINDING
                NavigationLink(destination: WelcomeHelpView(), isActive: $isHelpViewActive) {
                     SettingsRowLabel(iconName: "questionmark.circle.fill", iconColor: .purple, text: "Welcome & Help")
                 }
            }
            .listStyle(InsetGroupedListStyle())
            .navigationTitle("WarpsignFix")
            .navigationBarTitleDisplayMode(.large)
            .navigationBarItems(trailing: Button("Done") {
                let saved = configStore.saveConfig()
                
                if saved {
                    NSLog("WarpsignFixUI: Config saved successfully")
                    
                    if hasChangedStrategy {
                        // Ensure alert is shown on the main thread
                        DispatchQueue.main.async {
                            showTerminationAlert()
                        }
                    } else {
                        // Dismiss if save was successful and no restart needed
                        NotificationCenter.default.post(name: NSNotification.Name("WSFDismissSettingsNotification"), object: nil)
                    }
                } else {
                    NSLog("WarpsignFixUI: Failed to save config")
                    // Ensure alert is shown on the main thread
                    DispatchQueue.main.async {
                        showSaveErrorAlert()
                    }
                }
            })
            .safeAreaInset(edge: .bottom) {
                VStack(spacing: 5) {
                    // Make the link tappable
                    if let url = URL(string: "https://github.com/teflocarbon") {
                        Link("by Teflocarbon", destination: url)
                            .font(.footnote)
                            .foregroundColor(.accentColor)
                    } else {
                        Text("by Teflocarbon")
                            .font(.footnote)
                            .foregroundColor(.accentColor)
                    }

                    if let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String {
                         Text("Version \(version)")
                             .font(.caption)
                             .foregroundColor(.gray)
                     }
                }
                .frame(maxWidth: .infinity)
                .padding(.bottom, 10)
            }
            .onAppear {
                if startInHelpView && !didPerformInitialHelpNavigation {
                    isHelpViewActive = true
                    didPerformInitialHelpNavigation = true
                    NSLog("WSFSwiftUISettings: Performing initial navigation to Help view.")
                }
            }
        }
    }
    
    private func showTerminationAlert() {
        let alert = UIAlertController(
            title: "Restart Required",
            message: "The changes you made require the app to restart. The app will now close completely. Please restart it manually.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in
            // Log the termination
            NSLog("WarpsignFixUI: Terminating app to apply configuration changes")
            
            // Add a slight delay to ensure the alert is dismissed before termination
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                exit(0)
            }
        })
        
        // Find the presenting view controller and present the alert
        if let rootViewController = UIApplication.shared.windows.first?.rootViewController {
            var topController = rootViewController
            while let presentedViewController = topController.presentedViewController {
                topController = presentedViewController
            }
            topController.present(alert, animated: true)
        }
    }
    
    private func showSaveErrorAlert() {
        let alert = UIAlertController(
            title: "Save Error",
            message: "Failed to save the configuration. Please try again.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        
        // Find the presenting view controller and present the alert
        if let rootViewController = UIApplication.shared.windows.first?.rootViewController {
            var topController = rootViewController
            while let presentedViewController = topController.presentedViewController {
                topController = presentedViewController
            }
            topController.present(alert, animated: true)
        }
    }
    
    init(startInHelpView: Bool = false) {
        self.startInHelpView = startInHelpView
    }
}

// MARK: - Detail Views

struct KeychainSettingsView: View {
    @ObservedObject var configStore: WSFConfigStore
    @Binding var hasChangedStrategy: Bool
    
    private let keychainExplanation = "The Keychain is a secure database used by apps to store sensitive information like passwords, keys, and certificates."
    private let keychainFooter = "None: Don\'t modify keychain behavior.\n\nTeam ID: Modify keychain requests with old Team ID with new Team ID. Generally fixes issues with keychain.\n\nApp Group: Modify keychain remapping old ID with first App Group in entitlements. Can fix some apps that behave oddly with keychain. Use with caution."

    var body: some View {
        List {
            Section {
                Text(keychainExplanation)
                    .font(.footnote) // Make explanation text smaller
                    .foregroundColor(.secondary) // Use secondary color
            }
            
            Section(header: Text("Keychain Remapping Strategy"), footer: Text(keychainFooter)) {
                Picker("Strategy", selection: $configStore.keychainRemapStrategy) {
                    Text("None").tag(0)
                    Text("Team ID").tag(1)
                    Text("App Group").tag(2)
                }
                .pickerStyle(MenuPickerStyle())
                .onChange(of: configStore.keychainRemapStrategy) { _ in
                    hasChangedStrategy = true // Mark that a strategy changed
                }
            }
        }
        .listStyle(InsetGroupedListStyle())
        .navigationTitle("Keychain")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct CloudKitSettingsView: View {
    @ObservedObject var configStore: WSFConfigStore
    @Binding var hasChangedStrategy: Bool
    
    private let cloudKitExplanation = "CloudKit is Apple's framework for storing app data in iCloud, enabling features like data syncing across devices."
    private let cloudKitFooter = "None: Don't modify CloudKit behavior.\n\nFirst Container: Always use the first CloudKit container for all requests. Simple and reliable fix for CloudKit issues.\n\nRemap: Remap CloudKit container identifiers using configured mappings. Advanced option.\n\nNullify: Completely disable CloudKit functionality. Last resort if app crashes with CloudKit errors."

    var body: some View {
        List {
            // Conditionally display warning banner based on ACTUAL availability
            if !configStore.isiCloudRemappingAvailable {
                WarningBanner(message: "CloudKit 'Remap' strategy is currently unavailable. We couldn't find a valid 'remappings.json'")
            }
            
            Section {
                Text(cloudKitExplanation)
                    .font(.footnote) // Make explanation text smaller
                    .foregroundColor(.secondary) // Use secondary color
            }
            
            Section(header: Text("CloudKit Strategy"), footer: Text(cloudKitFooter)) {
                Picker("Strategy", selection: $configStore.cloudKitStrategy) {
                    Text("None").tag(0)
                    Text("First Container").tag(1)
                    // Conditionally show Remap option
                    if configStore.isiCloudRemappingAvailable {
                        Text("Remap").tag(2)
                    }
                    Text("Nullify").tag(3)
                }
                .pickerStyle(MenuPickerStyle())
                .onChange(of: configStore.cloudKitStrategy) { _ in 
                    // No need to check for invalid Remap selection if it's not shown
                    hasChangedStrategy = true // Mark that a strategy changed
                }
            }
        }
        .listStyle(InsetGroupedListStyle())
        .navigationTitle("CloudKit")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct AppGroupSettingsView: View {
    @ObservedObject var configStore: WSFConfigStore
    @Binding var hasChangedStrategy: Bool
    
    private let appGroupExplanation = "App Groups allow multiple apps from the same developer to share data through a common container directory (like shared settings or files)."
    private let appGroupFooter = "None: Don't modify App Group behavior.\n\nFirst Group: Use first valid App Group for invalid requests.\n\nFake Group: Use Documents directory for invalid groups.\n\nRemap: Use specific mappings for App Groups."

    var body: some View {
        List {
            // Conditionally display warning banner based on ACTUAL availability
            if !configStore.isAppGroupRemappingAvailable {
                 WarningBanner(message: "App Group 'Remap' strategy is currently unavailable. We couldn't find a valid 'remappings.json'")
            }
            
            Section {
                Text(appGroupExplanation)
                    .font(.footnote) // Make explanation text smaller
                    .foregroundColor(.secondary) // Use secondary color
            }
            
            Section(header: Text("App Group Strategy"), footer: Text(appGroupFooter)) {
                Picker("Strategy", selection: $configStore.appGroupStrategy) {
                    Text("None").tag(0)
                    Text("First Group").tag(1)
                    Text("Fake Group").tag(2)
                    // Conditionally show Remap option
                    if configStore.isAppGroupRemappingAvailable {
                        Text("Remap").tag(3)
                    }
                }
                .pickerStyle(MenuPickerStyle())
                .onChange(of: configStore.appGroupStrategy) { _ in
                   // No need to check for invalid Remap selection if it's not shown
                   hasChangedStrategy = true // Mark that a strategy changed
                }
            }
        }
        .listStyle(InsetGroupedListStyle())
        .navigationTitle("App Groups")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct DiagnosticsSettingsView: View {
    @ObservedObject var configStore: WSFConfigStore

    private let loggingFooter = "Enabling logging will create additional console output to help diagnose issues."
    private let welcomeFlagKey = "WarpsignFixHasLaunchedBefore"

    var body: some View {
        List {
            Section(header: Text("Logging"), footer: Text(loggingFooter)) {
                Toggle(isOn: $configStore.loggingEnabled) {
                    Text("Enable Logging")
                }
            }
            
            Section(header: Text("Debug Options")) {
                Button("Reset Welcome Flag") {
                    resetWelcomeFlag()
                }
                
                // Replace Debug Toggle with Picker
                Picker("Remapping Availability", selection: $configStore.debugRemappingOverride) {
                    Text("Default").tag(0)          // WSFDebugRemappingOverrideDefault
                    Text("Force Available").tag(1) // WSFDebugRemappingOverrideForceAvailable
                    Text("Force Unavailable").tag(2) // WSFDebugRemappingOverrideForceUnavailable
                }
                .pickerStyle(MenuPickerStyle()) // Or another style if preferred
                /*
                // Old Debug Toggle
                Toggle(isOn: $configStore.debugRemappingOverride) {
                    Text("Force Remapping Unavailable")
                }
                */
            }
        }
        .listStyle(InsetGroupedListStyle())
        .navigationTitle("Diagnostics")
        .navigationBarTitleDisplayMode(.inline)
    }
    
    private func resetWelcomeFlag() {
        UserDefaults.standard.removeObject(forKey: welcomeFlagKey)
        UserDefaults.standard.synchronize()
        NSLog("WarpsignFixUI: Welcome flag reset.")
        // Optionally, show an alert here to confirm to the user
    }
}

// MARK: - Helper Views

// Reusable Warning Banner View
struct WarningBanner: View {
    let message: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundColor(.white)
            Text(message)
                .font(.footnote)
                .foregroundColor(.white)
            Spacer() // Push content to the left
        }
        .padding(8)
        .background(Color.orange) // Changed to orange for better visibility
        .cornerRadius(8)
        .listRowInsets(EdgeInsets()) // Remove default list row padding
        .listRowBackground(Color.clear) // Make background transparent if needed within list
    }
}

@objcMembers
public class WSFConfigStore: NSObject, ObservableObject {
    @Published public var keychainRemapStrategy: Int {
        didSet {
            if oldValue != keychainRemapStrategy {
                let config = WSFConfig.shared()
                config.keychainRemapStrategy = WSFKeychainRemapStrategy(rawValue: keychainRemapStrategy) ?? .teamID
            }
        }
    }
    
    @Published public var cloudKitStrategy: Int {
        didSet {
            if oldValue != cloudKitStrategy {
                let config = WSFConfig.shared()
                config.cloudKitStrategy = WSFCloudKitStrategy(rawValue: cloudKitStrategy) ?? .none
            }
        }
    }
    
    @Published public var appGroupStrategy: Int {
        didSet {
            if oldValue != appGroupStrategy {
                let config = WSFConfig.shared()
                config.appGroupStrategy = WSFAppGroupStrategy(rawValue: appGroupStrategy) ?? .none
            }
        }
    }
    
    @Published public var loggingEnabled: Bool {
        didSet {
            if oldValue != loggingEnabled {
                let config = WSFConfig.shared()
                config.loggingEnabled = loggingEnabled
            }
        }
    }
    
    @Published public var debugRemappingOverride: Int {
         didSet {
             if oldValue != debugRemappingOverride {
                 // Ensure the value corresponds to a valid enum case before saving
                 if let newOverride = WSFDebugRemappingOverride(rawValue: debugRemappingOverride) {
                     WSFConfig.shared().debugRemappingOverride = newOverride
                 } else {
                     // Fallback or log error if somehow an invalid value is set
                     WSFConfig.shared().debugRemappingOverride = .default
                     // Optionally update the published value back to 0
                     // DispatchQueue.main.async { self.debugRemappingOverride = 0 }
                 }
                 objectWillChange.send() // Notify views of change
             }
         }
     }
    
    public static let shared = WSFConfigStore()
    
    private override init() {
        let config = WSFConfig.shared()
        self.keychainRemapStrategy = Int(config.keychainRemapStrategy.rawValue)
        self.cloudKitStrategy = Int(config.cloudKitStrategy.rawValue)
        self.appGroupStrategy = Int(config.appGroupStrategy.rawValue)
        self.loggingEnabled = config.loggingEnabled
        self.debugRemappingOverride = Int(config.debugRemappingOverride.rawValue) // Initialize new property
        super.init()
    }
    
    // Expose remappings file status
    public var remappingsFileLoadedSuccessfully: Bool {
        WSFConfig.shared().remappingsFileLoadedSuccessfully
    }
    
    // Check if iCloud Remapping is *actually* available considering debug override and data
    public var isiCloudRemappingAvailable: Bool {
        let overrideSetting = WSFDebugRemappingOverride(rawValue: debugRemappingOverride)
        var available: Bool
        
        switch overrideSetting {
            case .forceAvailable:
                available = true
            case .forceUnavailable:
                available = false
            case .default, .none: // Default case handles .default and potential nil/invalid rawValue
                available = remappingsFileLoadedSuccessfully && (WSFConfig.shared().iCloudContainerRemappings?.count ?? 0 > 0)
        }
        
        NSLog("[WSFConfigStore] isiCloudRemappingAvailable: Override = %ld, FileLoaded = %d, DataCount = %ld -> Returning %d",
              debugRemappingOverride,
              remappingsFileLoadedSuccessfully,
              WSFConfig.shared().iCloudContainerRemappings?.count ?? 0,
              available)
              
        return available
    }
    
    // Check if AppGroup Remapping is *actually* available considering debug override and data
    public var isAppGroupRemappingAvailable: Bool {
        let overrideSetting = WSFDebugRemappingOverride(rawValue: debugRemappingOverride)
        var available: Bool
        
        switch overrideSetting {
            case .forceAvailable:
                available = true
            case .forceUnavailable:
                available = false
            case .default, .none: // Default case handles .default and potential nil/invalid rawValue
                available = remappingsFileLoadedSuccessfully && (WSFConfig.shared().appGroupRemappings?.count ?? 0 > 0)
        }
        
        NSLog("[WSFConfigStore] isAppGroupRemappingAvailable: Override = %ld, FileLoaded = %d, DataCount = %ld -> Returning %d",
              debugRemappingOverride,
              remappingsFileLoadedSuccessfully,
              WSFConfig.shared().appGroupRemappings?.count ?? 0,
              available)
              
        return available
    }
    
    @objc public func saveConfig() -> Bool {
        let config = WSFConfig.shared()
        return config.save()
    }
}
