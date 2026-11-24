#import "WSFUtils.h"
#import <objc/runtime.h> // For NSClassFromString
#import <UIKit/UIKit.h>   // For UIApplication, UISceneDelegate etc.
#import "WSFConfigScreen.h" // To present the config panel

// --- Constants ---
static NSString * const kShortcutTypeShowSettingsDynamic = @"com.yourdomain.warpsignfix.showsettings.dynamic";

// --- Typedefs and pointers for storing original method implementations ---
typedef void (*AppDelegatePerformActionIMP)(id, SEL, UIApplication*, UIApplicationShortcutItem*, void (^)(BOOL));
static AppDelegatePerformActionIMP original_AppDelegate_performAction = NULL;
typedef void (*SceneDelegatePerformActionIMP)(id, SEL, UIWindowScene*, UIApplicationShortcutItem*, void (^)(BOOL));
static SceneDelegatePerformActionIMP original_SceneDelegate_performAction = NULL;


// --- Replacement function for AppDelegate performAction ---
static void replacement_AppDelegate_performAction(id self, SEL _cmd, UIApplication* application, UIApplicationShortcutItem* shortcutItem, void (^completionHandler)(BOOL)) {
     NSLog(@"[WarpsignFixHook] *** AppDelegate replacement_performAction called: %@ ***", shortcutItem.type);
    if ([shortcutItem.type isEqualToString:kShortcutTypeShowSettingsDynamic]) {
         NSLog(@"[WarpsignFixHook] Handling WarpsignFix 'Show Settings' shortcut (AppDelegate).");
        dispatch_async(dispatch_get_main_queue(), ^{
            [[WSFConfigScreen sharedInstance] presentConfigPanel];
        });
        completionHandler(YES); // We handled it
    } else {
        if (original_AppDelegate_performAction) {
             NSLog(@"[WarpsignFixHook] Passing unrecognized shortcut to original AppDelegate performAction impl.");
            original_AppDelegate_performAction(self, _cmd, application, shortcutItem, completionHandler);
        } else {
              NSLog(@"[WarpsignFixHook] Original AppDelegate performAction impl not found for unrecognized shortcut.");
             completionHandler(NO);
        }
    }
}

// --- Replacement function for SceneDelegate performAction ---
static void replacement_SceneDelegate_performAction(id self, SEL _cmd, UIWindowScene* windowScene, UIApplicationShortcutItem* shortcutItem, void (^completionHandler)(BOOL)) {
     NSLog(@"[WarpsignFixHook] *** SceneDelegate replacement_performAction called: %@ ***", shortcutItem.type);
    if ([shortcutItem.type isEqualToString:kShortcutTypeShowSettingsDynamic]) {
         NSLog(@"[WarpsignFixHook] Handling WarpsignFix 'Show Settings' shortcut (SceneDelegate).");
        dispatch_async(dispatch_get_main_queue(), ^{
            [[WSFConfigScreen sharedInstance] presentConfigPanel];
        });
        completionHandler(YES); // We handled it
    } else {
        if (original_SceneDelegate_performAction) {
             NSLog(@"[WarpsignFixHook] Passing unrecognized shortcut to original SceneDelegate performAction impl.");
            original_SceneDelegate_performAction(self, _cmd, windowScene, shortcutItem, completionHandler);
        } else {
             NSLog(@"[WarpsignFixHook] Original SceneDelegate performAction impl not found for unrecognized shortcut.");
             completionHandler(NO);
        }
    }
}


@implementation WSFUtils

+ (BOOL)isAirplaneModeEnabled {
    // CoreTelephony is another historical location. In my case, I used AppSupport.framework
    
    NSBundle *bundle = [NSBundle bundleWithPath:@"/System/Library/PrivateFrameworks/AppSupport.framework"];
    BOOL success = [bundle load];
    BOOL airplaneEnabled = NO; // Default to NO

    if (success) {
        Class RadiosPreferences = NSClassFromString(@"RadiosPreferences");
        if (RadiosPreferences) {
            // ARC will handle release if enabled
            id radioPreferences = [[RadiosPreferences alloc] init];
            
            if (radioPreferences) {
                // Use SEL directly if possible, slightly safer than string + respondsToSelector
                SEL selector = @selector(airplaneMode); // Standard selector name
                // Check respondsToSelector for robustness across iOS versions
                if ([radioPreferences respondsToSelector:selector]) {
                    // Using valueForKey works but direct property access might too if you declare a protocol
                    id value = [radioPreferences valueForKey:@"airplaneMode"];
                    if (value && [value isKindOfClass:[NSNumber class]]) {
                        airplaneEnabled = [value boolValue];
                         NSLog(@"[WSFUtils] Airplane mode checked via valueForKey: %@", airplaneEnabled ? @"ENABLED" : @"DISABLED");
                    } else {
                        NSLog(@"[WSFUtils] Warning: valueForKey:@\"airplaneMode\" returned unexpected value: %@", value);
                    }
                } else {
                     NSLog(@"[WSFUtils] Error: RadiosPreferences instance does not respond to airplaneMode selector.");
                }
            } else {
                 NSLog(@"[WSFUtils] Error: Failed to initialize RadiosPreferences instance.");
            }
        } else {
             NSLog(@"[WSFUtils] Error: RadiosPreferences private class not found.");
        }
    } else {
        NSLog(@"[WSFUtils] Error: Failed to load AppSupport.framework bundle.");
    }

    return airplaneEnabled;
}

+ (void)setupSceneDelegateShortcutHandling:(UIWindowScene *)windowScene {
     NSLog(@"[WarpsignFixSetup] Attempting to find and hook SceneDelegate for provided scene: %@", windowScene);
    Class sceneDelegateClass = nil;
    id sceneDelegateInstance = nil;

    if (@available(iOS 13.0, *)) {
        if (windowScene && [windowScene.delegate conformsToProtocol:@protocol(UIWindowSceneDelegate)]) {
            sceneDelegateInstance = windowScene.delegate;
            sceneDelegateClass = [sceneDelegateInstance class];
             NSLog(@"[WarpsignFixSetup] Found SceneDelegate instance from provided scene: %@, Class: %@", sceneDelegateInstance, NSStringFromClass(sceneDelegateClass));
        } else if (windowScene) {
             NSLog(@"[WarpsignFixSetup] Warning: Provided scene's delegate doesn't conform to UIWindowSceneDelegate or is nil.");
            if (windowScene.delegate) {
                sceneDelegateInstance = windowScene.delegate; // Attempt to get it anyway for logging/potential partial hook
                sceneDelegateClass = [windowScene.delegate class];
            }
        } else {
             NSLog(@"[WarpsignFixSetup] Warning: Provided windowScene is nil.");
        }
    } else {
         NSLog(@"[WarpsignFixSetup] iOS version < 13.0, SceneDelegate hook not applicable.");
    }

    if (sceneDelegateClass) {
        SEL selectorPerformActionScene = @selector(windowScene:performActionForShortcutItem:completionHandler:);
        Method originalMethodPerformActionScene = class_getInstanceMethod(sceneDelegateClass, selectorPerformActionScene);

        if (originalMethodPerformActionScene) {
            NSLog(@"[WarpsignFixSetup] Hooking existing SceneDelegate performAction...");
            original_SceneDelegate_performAction = (SceneDelegatePerformActionIMP)method_setImplementation(originalMethodPerformActionScene, (IMP)replacement_SceneDelegate_performAction);
            NSLog(@"[WarpsignFixSetup] Successfully hooked existing SceneDelegate performAction.");
        } else {
            NSLog(@"[WarpsignFixSetup] Adding SceneDelegate performAction dynamically...");
            const char *typeEncodingScene = "v@:@@@?";
            BOOL success = class_addMethod(sceneDelegateClass, selectorPerformActionScene, (IMP)replacement_SceneDelegate_performAction, typeEncodingScene);
            if (success) {
                original_SceneDelegate_performAction = NULL;
                NSLog(@"[WarpsignFixSetup] Successfully ADDED SceneDelegate performAction.");
            } else {
                NSLog(@"[WarpsignFixSetup] ERROR: Failed to add SceneDelegate performAction method dynamically.");
            }
        }
    } else {
        NSLog(@"[WarpsignFixSetup] Warning: Could not determine SceneDelegate class. Scene-based shortcut handling will fail.");
    }
    NSLog(@"[WarpsignFixSetup] Finished SceneDelegate hook setup attempt.");
}

+ (void)setupAppDelegateShortcutHandling:(UIApplication *)application {
      NSLog(@"[WarpsignFixSetup] Setting up AppDelegate hook/add for provided application: %@", application);
     id appDelegate = application.delegate;
     if (appDelegate) {
         Class appDelegateClass = [appDelegate class];
          NSLog(@"[WarpsignFixSetup] AppDelegate Class: %@", NSStringFromClass(appDelegateClass));
         SEL selectorPerformActionApp = @selector(application:performActionForShortcutItem:completionHandler:);
         Method originalMethodPerformActionApp = class_getInstanceMethod(appDelegateClass, selectorPerformActionApp);

         if (originalMethodPerformActionApp) {
               NSLog(@"[WarpsignFixSetup] Hooking existing AppDelegate performAction...");
              original_AppDelegate_performAction = (AppDelegatePerformActionIMP)method_setImplementation(originalMethodPerformActionApp, (IMP)replacement_AppDelegate_performAction);
               NSLog(@"[WarpsignFixSetup] Successfully hooked existing AppDelegate performAction.");
         } else {
               NSLog(@"[WarpsignFixSetup] Adding AppDelegate performAction dynamically...");
              const char *typeEncodingApp = "v@:@@@?";
              if (class_addMethod(appDelegateClass, selectorPerformActionApp, (IMP)replacement_AppDelegate_performAction, typeEncodingApp)) {
                  original_AppDelegate_performAction = NULL;
                   NSLog(@"[WarpsignFixSetup] Successfully ADDED AppDelegate performAction.");
              } else {
                   NSLog(@"[WarpsignFixSetup] ERROR: Failed to add AppDelegate performAction.");
              }
         }
     } else {
         NSLog(@"[WarpsignFixSetup] ERROR: AppDelegate instance is nil for provided application.");
     }
      NSLog(@"[WarpsignFixSetup] Finished AppDelegate hook setup.");
}


+ (void)registerDynamicSettingsShortcut {
     NSLog(@"[WarpsignFixSetup] Registering dynamic shortcut item if needed.");
    @try {
        UIApplication *sharedApp = [UIApplication sharedApplication];
        if (sharedApp) {
            BOOL shortcutExists = NO;
            for (UIApplicationShortcutItem *item in sharedApp.shortcutItems) {
                if ([item.type isEqualToString:kShortcutTypeShowSettingsDynamic]) {
                    shortcutExists = YES;
                     NSLog(@"[WarpsignFixSetup] Settings shortcut already exists.");
                    break;
                }
            }

            if (!shortcutExists) {
                 NSLog(@"[WarpsignFixSetup] Settings shortcut not found, adding it.");
                UIApplicationShortcutItem *settingsItem =
                    [[UIApplicationShortcutItem alloc] initWithType:kShortcutTypeShowSettingsDynamic
                                                     localizedTitle:@"WarpsignFix Settings"
                                                  localizedSubtitle:@"Configure fix strategies"
                                                               icon:[UIApplicationShortcutIcon iconWithType:9]
                                                           userInfo:nil];

                // Add our shortcut to the existing list
                NSMutableArray *mutableShortcutItems = [sharedApp.shortcutItems mutableCopy];
                if (!mutableShortcutItems) { // In case shortcutItems was nil
                    mutableShortcutItems = [NSMutableArray array];
                }
                [mutableShortcutItems addObject:settingsItem];
                sharedApp.shortcutItems = [mutableShortcutItems copy]; // Assign the updated array back
                 NSLog(@"[WarpsignFixSetup] Settings dynamic shortcut item added.");
            }
        } else {
             NSLog(@"[WarpsignFixSetup] Warning: UIApplication instance unexpectedly nil during shortcut registration.");
        }
    } @catch (NSException *exception) {
         NSLog(@"[WarpsignFixSetup] ***** EXCEPTION during dynamic shortcut registration: %@ *****", exception);
    }
     NSLog(@"[WarpsignFixSetup] Finished dynamic shortcut registration check/add.");
}


@end
