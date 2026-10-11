import SwiftUI

@main
struct WarpSignApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            TabView {
                AppsView()
                    .tabItem { Label("Apps", systemImage: "app.badge") }
                ProfilesView()
                    .tabItem { Label("Profiles", systemImage: "doc.badge.gearshape") }
            }
            .environment(model)
            // IPAs and profiles opened in WarpSign from Files or the share sheet
            .onOpenURL { url in
                Task {
                    if url.pathExtension.lowercased() == "mobileprovision" {
                        await model.addProfile(url)
                    } else {
                        await model.inspect(url)
                    }
                }
            }
        }
    }
}
