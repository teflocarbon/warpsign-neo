import SwiftUI

@main
struct WarpSignApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(model)
                // IPAs opened in WarpSign from Files or the share sheet
                .onOpenURL { url in
                    Task { await model.inspect(url) }
                }
        }
    }
}
