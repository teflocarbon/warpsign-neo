#import "WSFConfigScreen.h"
#import "WSFConfig.h"
#import <objc/runtime.h> // For dispatch_block_cancel if needed, though basic reset works too

// No need for @import directive, we'll use the Swift-generated header
#import <WarpsignFixUI/WarpsignFixUI-Swift.h>

@interface WSFConfigScreen ()

@property (nonatomic, strong) UIView *dotIndicator;
@property (nonatomic, strong) UIViewController *configViewController;
// Removed keyWindow property, retrieve it when needed
@property (nonatomic, assign) BOOL isPanelPresented;

// Property to manage the auto-hide timer/block
@property (nonatomic, strong) dispatch_block_t hideDotBlock;

@end

@implementation WSFConfigScreen

+ (instancetype)sharedInstance {
    static WSFConfigScreen *sharedInstance = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        sharedInstance = [[self alloc] init];
    });
    return sharedInstance;
}

- (instancetype)init {
    if (self = [super init]) {
        _isPanelPresented = NO;

        // Add observer for SwiftUI dismiss notification
        [[NSNotificationCenter defaultCenter] addObserver:self
                                                 selector:@selector(closeConfigPanel)
                                                     name:@"WSFDismissSettingsNotification"
                                                   object:nil];
        NSLog(@"WarpsignFixUI: WSFConfigScreen initialized and observers added.");
    }
    return self;
}

- (void)dealloc {
    // Cancel any pending hide block on dealloc
    if (self.hideDotBlock) {
        dispatch_block_cancel(self.hideDotBlock);
        self.hideDotBlock = nil;
    }
    [[NSNotificationCenter defaultCenter] removeObserver:self];
     NSLog(@"WarpsignFixUI: WSFConfigScreen deallocated and observers removed.");
}

// Helper method to get the key window reliably
- (UIWindow *)findKeyWindow {
    NSLog(@"WarpsignFixUI: Finding key window");
    UIWindow *foundWindow = nil;
    if (@available(iOS 13.0, *)) {
        for (UIWindowScene *windowScene in [UIApplication sharedApplication].connectedScenes) {
            if (windowScene.activationState == UISceneActivationStateForegroundActive) {
                 NSLog(@"WarpsignFixUI: Found active window scene: %@", windowScene.title ?: @"(no title)");
                // Prefer key window, but fall back to any foreground window if needed
                for (UIWindow *window in windowScene.windows) {
                    if (window.isKeyWindow) {
                        NSLog(@"WarpsignFixUI: Found key window in scene: %@", window);
                        foundWindow = window;
                        break; // Found key window, stop searching this scene
                    }
                    // Keep track of a potential non-key window just in case
                    if (!foundWindow && !window.hidden) {
                       foundWindow = window;
                    }
                }
                if (foundWindow) break; // Stop searching scenes if we found one
            }
        }
    }

    // Fallback for iOS 12 or if no key window found in scenes
    if (!foundWindow) {
        foundWindow = [UIApplication sharedApplication].keyWindow;
        if(foundWindow) {
             NSLog(@"WarpsignFixUI: Found key window using fallback method: %@", foundWindow);
        }
    }

     // Last resort: Iterate through all windows if still not found
    if (!foundWindow) {
        NSLog(@"WarpsignFixUI: Key window finding fallbacks exhausted, iterating all app windows.");
        for (UIWindow *window in [UIApplication sharedApplication].windows) {
             if (!window.hidden) {
                 foundWindow = window;
                 NSLog(@"WarpsignFixUI: Found a visible window as last resort: %@", foundWindow);
                 break;
             }
        }
    }


    if (!foundWindow) {
        NSLog(@"WarpsignFixUI: ERROR - No suitable window found to add the dot to.");
    }
    return foundWindow;
}


/// Ensures the configuration dot is visible and schedules it to hide.
/// Assumes the decision to show it (e.g., based on Airplane Mode) has already been made.
- (void)showConfigDot {
    NSLog(@"WarpsignFixUI: showConfigDot called.");

    dispatch_async(dispatch_get_main_queue(), ^{
        // --- Create Dot If Needed ---
        if (!self.dotIndicator) {
            NSLog(@"WarpsignFixUI: Creating dot indicator view.");
            // Make it slightly larger as discussed
            const CGFloat dotSize = 35.0; // Increased size
            self.dotIndicator = [[UIView alloc] initWithFrame:CGRectMake(15, 45, dotSize, dotSize)]; // Adjusted position slightly
            self.dotIndicator.backgroundColor = [UIColor orangeColor];
            self.dotIndicator.layer.cornerRadius = dotSize / 2.0;
            self.dotIndicator.layer.shadowColor = [UIColor blackColor].CGColor;
            self.dotIndicator.layer.shadowOffset = CGSizeMake(0, 2);
            self.dotIndicator.layer.shadowOpacity = 0.6; // Slightly stronger shadow
            self.dotIndicator.layer.shadowRadius = 5.0;
            self.dotIndicator.alpha = 0.0; // Start invisible before animation

            UITapGestureRecognizer *tapGesture = [[UITapGestureRecognizer alloc] initWithTarget:self action:@selector(dotTapped:)];
            [self.dotIndicator addGestureRecognizer:tapGesture];
            self.dotIndicator.userInteractionEnabled = YES;
            NSLog(@"WarpsignFixUI: Dot created with frame: %@.", NSStringFromCGRect(self.dotIndicator.frame));
        }

        // --- Add to Window If Needed ---
        UIWindow *window = [self findKeyWindow];
        if (!window) {
            NSLog(@"WarpsignFixUI: ERROR - Cannot show dot, no key window found.");
            return; // Cannot proceed without a window
        }

        if (![self.dotIndicator isDescendantOfView:window]) {
             NSLog(@"WarpsignFixUI: Adding dot to window: %@", window);
            [window addSubview:self.dotIndicator];
        } else {
            // Optional but good practice: Ensure it's on top if already added
             NSLog(@"WarpsignFixUI: Dot already in window, bringing to front.");
            [window bringSubviewToFront:self.dotIndicator];
        }

        // --- Cancel Previous Hide Timer & Animate In ---
        if (self.hideDotBlock) {
             NSLog(@"WarpsignFixUI: Cancelling previous hide timer.");
            dispatch_block_cancel(self.hideDotBlock);
            self.hideDotBlock = nil;
        }

        NSLog(@"WarpsignFixUI: Animating dot to visible.");
        [UIView animateWithDuration:0.3 animations:^{
            self.dotIndicator.alpha = 0.85; // Target visible alpha
        }];

        // --- Schedule New Hide Timer ---
        // Make the block weak-strong dance to avoid retain cycles
        __weak typeof(self) weakSelf = self;
        // DISPATCH_BLOCK_INHERIT_QOS_CLASS ensures the block runs with appropriate QoS
        self.hideDotBlock = dispatch_block_create(DISPATCH_BLOCK_INHERIT_QOS_CLASS, ^{
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf) {
                 NSLog(@"WarpsignFixUI: Auto-hiding dot after timeout.");
                 // Check if dot still exists and alpha isn't already 0 before animating
                 if (strongSelf.dotIndicator && strongSelf.dotIndicator.alpha > 0.0) {
                     [UIView animateWithDuration:0.3 animations:^{
                         strongSelf.dotIndicator.alpha = 0.0;
                     } completion:^(BOOL finished) {
                         // Optional: Remove from superview when hidden to clean up?
                         // Or just leave it there with alpha 0 for faster reuse.
                         // If removing:
                         // if (finished) {
                         //    [strongSelf.dotIndicator removeFromSuperview];
                         // }
                         NSLog(@"WarpsignFixUI: Dot fade-out animation completed.");
                     }];
                 } else {
                     NSLog(@"WarpsignFixUI: Auto-hide timer fired but dot was already hidden or gone.");
                 }
                strongSelf.hideDotBlock = nil; // Clear the block reference once executed
            }
        });

        // Schedule the block
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(8 * NSEC_PER_SEC)), dispatch_get_main_queue(), self.hideDotBlock);
         NSLog(@"WarpsignFixUI: Scheduled dot auto-hide in 8 seconds.");
    });
}

/// Immediately hides the configuration dot (e.g., if Airplane Mode turns off).
- (void)hideConfigDot {
    NSLog(@"WarpsignFixUI: hideConfigDot called.");
    dispatch_async(dispatch_get_main_queue(), ^{
        // Cancel any pending auto-hide timer
        if (self.hideDotBlock) {
            NSLog(@"WarpsignFixUI: Cancelling pending hide timer because hideConfigDot was called.");
            dispatch_block_cancel(self.hideDotBlock);
            self.hideDotBlock = nil;
        }

        // Animate out if the dot exists and is visible
        if (self.dotIndicator && self.dotIndicator.alpha > 0.0) {
            NSLog(@"WarpsignFixUI: Animating dot to hidden.");
            [UIView animateWithDuration:0.3 animations:^{
                self.dotIndicator.alpha = 0.0;
            } completion:^(BOOL finished) {
                // Optional: Remove from superview? See comment in showConfigDot.
                NSLog(@"WarpsignFixUI: Dot hide animation completed.");
            }];
        } else {
             NSLog(@"WarpsignFixUI: Dot already hidden or does not exist.");
        }
    });
}


- (void)dotTapped:(UITapGestureRecognizer *)recognizer {
    NSLog(@"WarpsignFixUI: Dot tapped.");

    // Cancel any pending auto-hide immediately
    if (self.hideDotBlock) {
         NSLog(@"WarpsignFixUI: Cancelling hide timer due to tap.");
        dispatch_block_cancel(self.hideDotBlock);
        self.hideDotBlock = nil;
    }

    // Hide the dot immediately (or with very short animation) when tapped
    [UIView animateWithDuration:0.15 animations:^{ // Faster fade out on tap
        self.dotIndicator.alpha = 0.0;
    }];

    // Present the SwiftUI config panel
    [self presentConfigPanel];
}

- (void)presentConfigPanel {
     NSLog(@"WarpsignFixUI: Attempting to present config panel.");

    if (self.isPanelPresented) {
         NSLog(@"WarpsignFixUI: Config panel already presented, ignoring request.");
        return;
    }

    // Ensure running on main thread
    dispatch_async(dispatch_get_main_queue(), ^{
        // --- Find Top VC FIRST ---
        NSLog(@"WarpsignFixUI: Finding top view controller for presentation.");
        UIViewController *topVC = [self topViewController];

        if (!topVC) {
            NSLog(@"WarpsignFixUI: ERROR - Failed to find a top view controller to present from. Aborting presentation.");
            // No need to reset isPanelPresented here as it hasn't been set to YES yet.
            return;
        }

        // --- Prepare Config VC ---
        NSLog(@"WarpsignFixUI: Creating SwiftUI config view controller.");
        UIViewController *configVC = [WSFSwiftUISettingsHostViewController createHostingControllerWithStartInHelpView:NO];

        // --- Set Delegate for Interactive Dismissal ---
        // Check availability if targeting older iOS, but .formSheet implies 13+
        if (@available(iOS 13.0, *)) {
            configVC.presentationController.delegate = self;
             NSLog(@"WarpsignFixUI: Set presentation controller delegate.");
        }

        // --- Set Flag and Present ---
        self.isPanelPresented = YES; // Set flag right before presentation attempt
        NSLog(@"WarpsignFixUI: Found top view controller: %@. Presenting config panel.", [topVC class]);

        [topVC presentViewController:configVC animated:YES completion:^{
             NSLog(@"WarpsignFixUI: Config panel presentation animation completed.");
             // Only set configViewController reference *after* successful presentation starts
             self.configViewController = configVC;
        }];

        // Note: Moved self.configViewController assignment into completion block
        // to ensure it's only set if presentation begins successfully.
        // However, for the delegate/cleanup logic, having it slightly earlier might be okay too.
        // Let's keep it in completion for now, but be mindful if cleanup needs it sooner.
        // Reconsidering: Set it *before* presentation to ensure delegate method can find it.
        self.configViewController = configVC; // Set reference before presenting

    });
}

// New method to present directly to the Help view
- (void)presentHelpPanel {
     NSLog(@"WarpsignFixUI: Attempting to present help panel directly.");

    if (self.isPanelPresented) {
         NSLog(@"WarpsignFixUI: Panel already presented, ignoring request to show help directly.");
        return;
    }

    // Ensure running on main thread
    dispatch_async(dispatch_get_main_queue(), ^{
        // --- Find Top VC FIRST ---
        NSLog(@"WarpsignFixUI: Finding top view controller for help presentation.");
        UIViewController *topVC = [self topViewController];

        if (!topVC) {
            NSLog(@"WarpsignFixUI: ERROR - Failed to find a top view controller to present help from. Aborting presentation.");
            return;
        }

        // --- Prepare Config VC (telling it to start in help) ---
        NSLog(@"WarpsignFixUI: Creating SwiftUI config view controller (starting in help).");
        UIViewController *configVC = [WSFSwiftUISettingsHostViewController createHostingControllerWithStartInHelpView:YES]; // <-- PASS YES HERE

        // --- Set Delegate for Interactive Dismissal ---
        if (@available(iOS 13.0, *)) {
            configVC.presentationController.delegate = self;
             NSLog(@"WarpsignFixUI: Set presentation controller delegate for help panel.");
        }

        // --- Set Flag and Present ---
        self.isPanelPresented = YES; // Set flag right before presentation attempt
        NSLog(@"WarpsignFixUI: Found top view controller: %@. Presenting help panel.", [topVC class]);

        [topVC presentViewController:configVC animated:YES completion:^{
             NSLog(@"WarpsignFixUI: Help panel presentation animation completed.");
             self.configViewController = configVC; // Assign reference after successful presentation starts
        }];

        self.configViewController = configVC; // Set reference before presenting for delegate
    });
}

// --- Start topViewController methods ---
- (UIViewController *)topViewController {
     NSLog(@"WarpsignFixUI: Finding top view controller.");
    UIWindow *window = [self findKeyWindow]; // Use helper method
    UIViewController *rootViewController = window.rootViewController;
    if (!rootViewController) {
         NSLog(@"WarpsignFixUI: ERROR - No root view controller found in window: %@", window);
        return nil;
    }
     NSLog(@"WarpsignFixUI: Root view controller is: %@", [rootViewController class]);
    return [self topViewControllerWithRootViewController:rootViewController];
}

- (UIViewController *)topViewControllerWithRootViewController:(UIViewController *)rootViewController {
    if ([rootViewController isKindOfClass:[UINavigationController class]]) {
        UINavigationController *navigationController = (UINavigationController *)rootViewController;
         NSLog(@"WarpsignFixUI: Traversing NavigationController, getting visibleViewController.");
        // Check if visibleViewController is nil (can happen during transitions)
        UIViewController *visibleVC = navigationController.visibleViewController;
        return visibleVC ? [self topViewControllerWithRootViewController:visibleVC] : navigationController; // Return nav controller itself if visible is nil? Or root? Check best practice. Let's recurse with visibleVC if available.
    }

    if ([rootViewController isKindOfClass:[UITabBarController class]]) {
        UITabBarController *tabController = (UITabBarController *)rootViewController;
         NSLog(@"WarpsignFixUI: Traversing TabBarController, getting selectedViewController.");
         UIViewController *selectedVC = tabController.selectedViewController;
        // SelectedVC could be nil if tab bar is empty or just loaded
        return selectedVC ? [self topViewControllerWithRootViewController:selectedVC] : tabController; // Recurse if selectedVC exists
    }

    if (rootViewController.presentedViewController) {
        NSLog(@"WarpsignFixUI: Found presentedViewController: %@. Traversing.", [rootViewController.presentedViewController class]);
        return [self topViewControllerWithRootViewController:rootViewController.presentedViewController];
    }

    NSLog(@"WarpsignFixUI: Reached top view controller: %@", [rootViewController class]);
    return rootViewController;
}
// --- End topViewController methods ---


- (void)closeConfigPanel {
     NSLog(@"WarpsignFixUI: closeConfigPanel called (likely from Done button notification).");
    // Ensure we have a view controller to dismiss and we think it's presented
    if (self.configViewController && self.isPanelPresented) {
        // Get a local reference in case dismiss completion is delayed and self.configViewController is cleared elsewhere
        UIViewController *controllerToDismiss = self.configViewController;
        // Clear the reference immediately
        self.configViewController = nil;
         // Set flag immediately - dismissal is starting
        self.isPanelPresented = NO;

        dispatch_async(dispatch_get_main_queue(), ^{
            NSLog(@"WarpsignFixUI: Dismissing config panel via closeConfigPanel: %@", controllerToDismiss);
            [controllerToDismiss dismissViewControllerAnimated:YES completion:^{
                NSLog(@"WarpsignFixUI: Config panel dismissal (from Done button) completed.");
                // State is already updated
            }];
        });
    } else {
         NSLog(@"WarpsignFixUI: closeConfigPanel: No config panel to dismiss or panel not marked as presented (configVC=%@, isPanelPresented=%@).", self.configViewController, self.isPanelPresented ? @"YES" : @"NO");
         // Force reset flag just in case state is inconsistent
         if (self.isPanelPresented) {
             self.isPanelPresented = NO;
         }
         if (self.configViewController) {
             self.configViewController = nil;
         }
    }
}

#pragma mark - UIAdaptivePresentationControllerDelegate

// Called when the sheet is dismissed interactively (e.g., by swiping down)
- (void)presentationControllerDidDismiss:(UIPresentationController *)presentationController {
    NSLog(@"WarpsignFixUI: presentationControllerDidDismiss called.");
    // Check if the dismissed controller is the one we manage
    if (presentationController.presentedViewController == self.configViewController) {
        NSLog(@"WarpsignFixUI: Interactive dismissal detected for config panel. Attempting to save...");

        BOOL saved = [[WSFConfigStore shared] saveConfig];
        if (saved) {
            NSLog(@"WarpsignFixUI: Config saved successfully on interactive dismiss.");
        } else {
            NSLog(@"WarpsignFixUI: ERROR - Failed to save config on interactive dismiss.");
        }

        NSLog(@"WarpsignFixUI: Resetting state after interactive dismissal.");
        self.configViewController = nil;
        self.isPanelPresented = NO; // Reset the flag
    } else {
        NSLog(@"WarpsignFixUI: Interactive dismissal detected, but not for the expected config panel. Ignoring.");
    }
}

@end
