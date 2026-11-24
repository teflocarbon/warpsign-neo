// WSFAppGroupFix.h
#import <Foundation/Foundation.h>
#import "WSFConfigLoader.h" // Import to get WSFAppGroupStrategy enum

@interface WSFAppGroupFix : NSObject

// --- Access to Configuration ---

// Returns the currently active strategy for handling App Groups.
+ (WSFAppGroupStrategy)currentAppGroupStrategy;

// Returns the specific App Group remappings loaded from configuration (only relevant for Remap strategy).
// Key: Original App Group ID, Value: Remapped App Group ID.
+ (NSDictionary *)appGroupMappings;

// Returns whether detailed logging for App Group fixing is enabled.
+ (BOOL)isLoggingEnabled;


// --- Access to Extracted Information ---

// Returns the first valid App Group ID found in the application's entitlements.
// Returns nil if no valid app groups are found.
+ (NSString *)firstAppGroupID;

// Returns an array of all App Group IDs found in the application's entitlements.
// Returns nil if entitlements couldn't be read or no groups were found.
+ (NSArray<NSString *> *)validAppGroupIDs;


// --- Diagnostic Helper Methods ---

// Logs information about the App Group entitlements found for the current application.
// (This method might simply call the internal extractAppGroupInfo if needed externally,
// or have its own simplified logging).
+ (void)checkAppGroupEntitlements; // Matches the pattern

@end
