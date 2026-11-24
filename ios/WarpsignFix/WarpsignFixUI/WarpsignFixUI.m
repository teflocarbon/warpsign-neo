//  WarpsignFixUI.m

#import "WarpsignFixUI.h"
#import "WSFConfigScreen.h"
#import "WSFUtils.h"
#import "WSFExternalConfig.h"
#import <UIKit/UIKit.h>

// --- Assumed Globals & Forward Declarations (Must be defined elsewhere in the file or included headers) ---
static BOOL g_warpsignFixSetupComplete = NO; // Flag to ensure setup runs only once
static dispatch_once_t g_warpsignFixSetupOnceToken; // Token for dispatch_once

// Tokens for ensuring setup runs only once per context (App vs Scene)
static dispatch_once_t g_appDelegateHookToken;
static dispatch_once_t g_sceneDelegateHookToken;

// Constants used within this function
static NSString * const kHasLaunchedBeforeKey = @"WarpsignFixHasLaunchedBefore";

// --- Notification Handlers ---

// Handler for UIApplicationDidFinishLaunchingNotification
static void handleAppDidFinishLaunching(NSNotification *notification) {
    NSLog(@"[WarpsignFixInit] Received UIApplicationDidFinishLaunchingNotification.");
    UIApplication *application = notification.object;
    if (application) {
        // Ensure hook setup runs only once for the App delegate
        dispatch_once(&g_appDelegateHookToken, ^{
            NSLog(@"[WarpsignFixInit] Setting up AppDelegate hook via notification.");
            [WSFUtils setupAppDelegateShortcutHandling:application];
        });
    } else {
        NSLog(@"[WarpsignFixInit] Warning: UIApplication instance is nil in launch notification.");
    }
}

// Handler for UISceneDidActivateNotification
static void handleSceneDidActivate(NSNotification *notification) {
    if (@available(iOS 13.0, *)) {
        NSLog(@"[WarpsignFixInit] Received UISceneDidActivateNotification.");
        UIScene *scene = notification.object;
        if ([scene isKindOfClass:[UIWindowScene class]]) {
            UIWindowScene *windowScene = (UIWindowScene *)scene;
            // Ensure hook setup runs only once for the Scene delegate
            // NOTE: This might need refinement if multiple scenes can be active
            // and need independent shortcut handling, but for a single-window app,
            // setting it up on the first activation is usually sufficient.
            dispatch_once(&g_sceneDelegateHookToken, ^{
                 NSLog(@"[WarpsignFixInit] Setting up SceneDelegate hook via notification for scene: %@", windowScene);
                [WSFUtils setupSceneDelegateShortcutHandling:windowScene];
            });
        } else {
            NSLog(@"[WarpsignFixInit] Received scene activation notification, but it's not a UIWindowScene: %@", scene);
        }
    }
}

// --- Main Setup Function (This performs the core setup after a delay) ---
static void performWarpsignFixDelayedSetup() {
    // Use dispatch_once to guarantee this setup logic runs exactly one time per app launch
    dispatch_once(&g_warpsignFixSetupOnceToken, ^{
         NSLog(@"[WarpsignFixSetup] Performing delayed setup...");

        // --- Setup SceneDelegate Hook/Add (Primary Target) ---
        // [WSFUtils setupSceneDelegateShortcutHandling];

        // --- Setup AppDelegate Hook/Add (Secondary Fallback) ---
        // [WSFUtils setupAppDelegateShortcutHandling];

        // --- Register Dynamic Shortcut ---
        [WSFUtils registerDynamicSettingsShortcut];

        // --- Perform First Launch Check & Alert ---
        NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
        BOOL hasLaunchedBefore = [defaults boolForKey:kHasLaunchedBeforeKey];
         NSLog(@"[WarpsignFixSetup] First launch check: %d", hasLaunchedBefore);
        if (!hasLaunchedBefore) {
             NSLog(@"[WarpsignFixSetup] First launch detected. Presenting welcome alert.");
             UIViewController *topVC = [[WSFConfigScreen sharedInstance] topViewController]; // Get top VC for presentation
            if (topVC) {
                 UIAlertController *alert = [UIAlertController alertControllerWithTitle:@"Welcome to WarpsignFix"
                                                                                message:@"Thank you for using WarpsignFix!\n\nIt\'s recommended to review the settings now.\n\nTap \'Show Help\' for details on accessing settings and troubleshooting."
                                                                         preferredStyle:UIAlertControllerStyleAlert];
                 [alert addAction:[UIAlertAction actionWithTitle:@"OK" style:UIAlertActionStyleDefault handler:nil]];
                 [alert addAction:[UIAlertAction actionWithTitle:@"Show Help" style:UIAlertActionStyleDefault handler:^(UIAlertAction * _Nonnull action) {
                     NSLog(@"[WarpsignFixSetup] 'Show Help' tapped from welcome alert.");
                     [WarpsignFixUI showHelpScreen]; // Call the new public method to show help directly
                 }]];
                 [topVC presentViewController:alert animated:YES completion:^{
                       NSLog(@"[WarpsignFixSetup] Welcome alert presented.");
                 }];
            } else {
                   NSLog(@"[WarpsignFixSetup] ERROR - Could not find top VC for welcome alert.");
            }
            // Mark that the first launch has occurred
            [defaults setBool:YES forKey:kHasLaunchedBeforeKey];
            [defaults synchronize];
               NSLog(@"[WarpsignFixSetup] First launch flag set.");
        }
        // --- End First Launch Check ---


        // --- Airplane Mode Dot Trigger (Optional Secondary Trigger) ---
        BOOL isAirplane = [WSFUtils isAirplaneModeEnabled];
        NSLog(@"[WarpsignFixSetup] Airplane Mode status: %@", isAirplane ? @"ENABLED" : @"DISABLED");
        if (isAirplane) {
             NSLog(@"[WarpsignFixSetup] Airplane Mode ON, showing config dot.");
             [[WSFConfigScreen sharedInstance] showConfigDot]; // Show the purple dot
        }
        // --- End Airplane Check ---

         NSLog(@"[WarpsignFixSetup] Delayed setup finished.");
         g_warpsignFixSetupComplete = YES; // Mark setup as complete
    }); // End of dispatch_once block
}

// --- Tweak Implementation ---
@implementation WarpsignFixUI

// Constructor - Minimal, just schedules the real setup
__attribute__((constructor))
static void initializeWarpsignFixUI_Constructor() {
    // --- Load external config VERY early ---
    [WSFExternalConfig loadAndUpdateUserDefaults];
    // --- End external config load ---

    NSLog(@"[WarpsignFixInit] Constructor started. Registering notification observers and scheduling delayed setup.");

    // --- Register Notification Observers ---
    [[NSNotificationCenter defaultCenter] addObserverForName:UIApplicationDidFinishLaunchingNotification
                                                      object:nil
                                                       queue:[NSOperationQueue mainQueue]
                                                  usingBlock:^(NSNotification * _Nonnull note) {
                                                      handleAppDidFinishLaunching(note);
                                                  }];

    if (@available(iOS 13.0, *)) {
        [[NSNotificationCenter defaultCenter] addObserverForName:UISceneDidActivateNotification
                                                          object:nil // Observe for any scene
                                                           queue:[NSOperationQueue mainQueue]
                                                      usingBlock:^(NSNotification * _Nonnull note) {
                                                          handleSceneDidActivate(note);
                                                      }];
    }
    // --- End Notification Observers ---

    // --- REMOVED Hook Setup from here ---
    // [WSFUtils setupSceneDelegateShortcutHandling]; // MOVED to notification
    // [WSFUtils setupAppDelegateShortcutHandling]; // MOVED to notification
    // --- End Removed Hook Setup ---

    // Schedule the rest of the setup function after a short delay (e.g., 1.5 second)
    // Adjusted delay slightly based on previous value.
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        performWarpsignFixDelayedSetup();
    });

     NSLog(@"[WarpsignFixInit] Constructor finished.");
}

// Method to show panel (can be called by handler)
+ (void)showConfigScreen {
     NSLog(@"[WarpsignFixUI] showConfigScreen called.");
    dispatch_async(dispatch_get_main_queue(), ^{
      [[WSFConfigScreen sharedInstance] presentConfigPanel];
    });
}

// Method to show help panel (can be called by handler)
+ (void)showHelpScreen {
     NSLog(@"[WarpsignFixUI] showHelpScreen called.");
    dispatch_async(dispatch_get_main_queue(), ^{
      [[WSFConfigScreen sharedInstance] presentHelpPanel]; // Call the specific presentation method
    });
}

@end // End WarpsignFixUI Implementation
