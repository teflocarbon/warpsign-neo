// WSFConfig.h
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

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

// Define App Group handling strategies
typedef NS_ENUM(NSInteger, WSFAppGroupStrategy) {
    WSFAppGroupStrategyNone = 0,        // Don't modify App Group behavior
    WSFAppGroupStrategyFirstGroup = 1,   // Use first valid App Group from entitlements
    WSFAppGroupStrategyFakeGroup = 2,    // Use Documents directory for invalid groups
    WSFAppGroupStrategyRemap = 3         // Use specific mappings for App Groups
};

// Debug override for remapping availability
typedef NS_ENUM(NSInteger, WSFDebugRemappingOverride) {
    WSFDebugRemappingOverrideDefault = 0,       // Use actual status
    WSFDebugRemappingOverrideForceAvailable = 1, // Force available
    WSFDebugRemappingOverrideForceUnavailable = 2 // Force unavailable
};

@interface WSFConfig : NSObject

@property (nonatomic, assign) WSFKeychainRemapStrategy keychainRemapStrategy;
@property (nonatomic, assign) BOOL loggingEnabled;

// CloudKit strategy and configuration
@property (nonatomic, assign) WSFCloudKitStrategy cloudKitStrategy;
// iCloud container remappings from JSON
@property (nonatomic, strong, nullable) NSArray *iCloudContainerRemappings;

// App Group strategy and configuration
@property (nonatomic, assign) WSFAppGroupStrategy appGroupStrategy;
// App Group remappings from JSON
@property (nonatomic, strong, nullable) NSArray *appGroupRemappings;

// Flag indicating if remappings.json was loaded successfully
@property (nonatomic, assign, readonly) BOOL remappingsFileLoadedSuccessfully;

// Debug override for remapping availability
@property (nonatomic, assign) WSFDebugRemappingOverride debugRemappingOverride;

// Singleton instance
+ (instancetype)sharedConfig;

// Load config from NSUserDefaults (or create default if not exists)
- (void)loadConfig;

// Save current config to NSUserDefaults
- (BOOL)saveConfig;

// Load remappings from JSON file
- (void)loadRemappingsFromBundle;

@end

NS_ASSUME_NONNULL_END
