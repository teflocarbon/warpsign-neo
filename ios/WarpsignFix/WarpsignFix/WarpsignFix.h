// WarpsignFix.h
#import <Foundation/Foundation.h>
#import "WSFConfigLoader.h" // Import to get the enums

@interface WarpsignFix : NSObject

// Entitlement checking
+ (void)checkAppGroupsEntitlement;
+ (void)checkCloudKitEntitlements;

// Configuration methods - Keychain
+ (WSFKeychainRemapStrategy)currentKeychainRemapStrategy;

// Configuration methods - CloudKit
+ (WSFCloudKitStrategy)currentCloudKitStrategy;
+ (NSDictionary *)cloudKitMappings;

// Configuration methods - App Groups
+ (WSFAppGroupStrategy)currentAppGroupStrategy;
+ (NSDictionary *)appGroupMappings;

// General Configuration
+ (void)loadConfiguration;
+ (BOOL)isLoggingEnabled;

@end
