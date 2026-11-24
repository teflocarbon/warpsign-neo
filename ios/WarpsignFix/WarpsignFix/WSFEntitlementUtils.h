#import <Foundation/Foundation.h>

@interface WSFEntitlementUtils : NSObject

// Returns all valid app group IDs from entitlements
+ (NSArray<NSString *> *)extractAppGroupIDs;

// Returns the first valid app group ID (preferring those starting with "group.")
+ (NSString *)extractFirstAppGroupID;

// Checks if a given group ID is present in the app's entitlements
+ (BOOL)isAppGroupIDValid:(NSString *)groupID;

// Returns whether logging is enabled (reads from WSFConfigLoader)
+ (BOOL)isLoggingEnabled;

@end 