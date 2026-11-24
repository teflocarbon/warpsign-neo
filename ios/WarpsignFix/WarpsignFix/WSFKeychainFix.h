// WSFKeychainFix.h
#import <Foundation/Foundation.h>
#import "WSFConfigLoader.h" // Import to get WSFKeychainRemapStrategy enum

@interface WSFKeychainFix : NSObject

// Access to the extracted identifiers
+ (NSString *)currentTeamID;
+ (NSString *)firstAppGroup;

// Configuration methods
+ (WSFKeychainRemapStrategy)currentKeychainRemapStrategy;
+ (BOOL)isLoggingEnabled;

// Helper methods
+ (void)checkAppGroupsEntitlement;

@end
