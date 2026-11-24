//  WarpsignFixUI.h

#import <Foundation/Foundation.h>

//! Project version number for WarpsignFixUI.
FOUNDATION_EXPORT double WarpsignFixUIVersionNumber;

//! Project version string for WarpsignFixUI.
FOUNDATION_EXPORT const unsigned char WarpsignFixUIVersionString[];

// Framework umbrella header

// Import all public headers of the framework
#import <WarpsignFixUI/WSFConfig.h>
#import <WarpsignFixUI/WSFConfigScreen.h>
// Swift compatibility will be handled by the auto-generated Swift interface
// <WarpsignFixUI/WarpsignFixUI-Swift.h>

@interface WarpsignFixUI : NSObject

// Method to manually show the configuration screen
+ (void)showConfigScreen;

// Method to manually show the configuration screen starting at the Help page
+ (void)showHelpScreen;

@end