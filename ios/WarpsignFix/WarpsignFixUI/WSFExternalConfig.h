// WSFExternalConfig.h
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface WSFExternalConfig : NSObject

/**
 * Helper method to get the full path to the configuration plist in the Documents directory.
 * Returns nil if the Documents directory cannot be found.
 */
+ (nullable NSString *)configPlistPath;

/**
 * Checks for warpsignfix_config.plist in the app's Documents directory.
 * If not found, copies warpsignfix_config_sample.plist from the bundle.
 * Reads the configuration from warpsignfix_config.plist and updates NSUserDefaults.
 * Should be called very early during initialization.
 */
+ (void)loadAndUpdateUserDefaults;

@end

NS_ASSUME_NONNULL_END 