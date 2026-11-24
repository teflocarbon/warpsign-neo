// WSFConfigLoader.h
// A minimal configuration loader for WarpsignFix

#import <Foundation/Foundation.h>

// Define keychain remap strategies
typedef NS_ENUM(NSInteger, WSFKeychainRemapStrategy) {
    WSFKeychainRemapStrategyNone = 0,      // Don't modify keychain behavior
    WSFKeychainRemapStrategyTeamID = 1,    // Use Team ID extraction method
    WSFKeychainRemapStrategyAppGroup = 2   // Use first app group for remapping
};

// Define CloudKit handling strategies - must match WSFCloudKitStrategy in WSFCloudKitFix.h
typedef NS_ENUM(NSInteger, WSFCloudKitStrategy) {
    WSFCloudKitStrategyNone = 0,            // Don't modify CloudKit behavior
    WSFCloudKitStrategyFirstContainer = 1,  // Use the first CloudKit container from entitlements
    WSFCloudKitStrategyRemap = 2,           // Remap CloudKit container identifiers using mappings
    WSFCloudKitStrategyNullify = 3          // Nullify CloudKit functionality
};

// Define App Group strategies
typedef NS_ENUM(NSInteger, WSFAppGroupStrategy) {
    WSFAppGroupStrategyNone = 0,        // Don't modify App Group behavior
    WSFAppGroupStrategyFirstGroup = 1,   // Use first valid App Group from entitlements
    WSFAppGroupStrategyFakeGroup = 2,    // Use Documents directory for invalid groups
    WSFAppGroupStrategyRemap = 3         // Use specific mappings for App Groups
};

@interface WSFConfigLoader : NSObject

// Load the keychain strategy from NSUserDefaults
+ (WSFKeychainRemapStrategy)loadKeychainRemapStrategy;

// Load the CloudKit strategy from NSUserDefaults
+ (WSFCloudKitStrategy)loadCloudKitStrategy;

// Load the App Group strategy from NSUserDefaults
+ (WSFAppGroupStrategy)loadAppGroupStrategy;

// Load iCloud container remappings from NSUserDefaults
+ (NSDictionary *)loadCloudKitMappings;

// Load App Group remappings from NSUserDefaults
+ (NSDictionary *)loadAppGroupMappings;

// Load logging enabled flag from NSUserDefaults
+ (BOOL)loadLoggingEnabled;

@end