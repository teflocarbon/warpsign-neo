#import <Foundation/Foundation.h>
#import <UIKit/UIKit.h>
#import <WarpsignFixUI/WSFConfig.h>

NS_ASSUME_NONNULL_BEGIN

@interface WSFConfigScreen : NSObject <UIAdaptivePresentationControllerDelegate>

+ (instancetype)sharedInstance;

/// Ensures the configuration dot is visible and schedules it to hide after 8 seconds.
/// Assumes the decision to show it (e.g., based on Airplane Mode) has already been made.
- (void)showConfigDot;

/// Immediately hides the configuration dot.
- (void)hideConfigDot;

/// Presents the configuration panel modally.
- (void)presentConfigPanel;

/// Dismisses the configuration panel if it's presented.
- (void)closeConfigPanel; // Added this based on your implementation

/// Presents the configuration panel modally, starting directly in the Help view.
- (void)presentHelpPanel; // <-- Added declaration

/// Finds the topmost view controller suitable for presenting modal views.
- (UIViewController * _Nullable)topViewController; // <-- Added declaration, marked nullable

@end

NS_ASSUME_NONNULL_END
