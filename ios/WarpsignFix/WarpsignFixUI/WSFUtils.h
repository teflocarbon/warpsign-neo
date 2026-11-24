#import <Foundation/Foundation.h>

@class UIApplication, UIWindowScene; // Forward declarations for UIKit types

@interface WSFUtils : NSObject

/// Checks if Airplane Mode is enabled using private APIs.
+ (BOOL)isAirplaneModeEnabled;

/// Sets up method hooking or addition for AppDelegate shortcut handling.
+ (void)setupAppDelegateShortcutHandling:(UIApplication *)application;

/// Sets up method hooking or addition for SceneDelegate shortcut handling.
+ (void)setupSceneDelegateShortcutHandling:(UIWindowScene *)windowScene;

/// Registers the dynamic shortcut item for accessing settings.
+ (void)registerDynamicSettingsShortcut;

@end
