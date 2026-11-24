// WSFConfigLoader.m
#import "WSFConfigLoader.h"

// Use the same NSUserDefaults keys for compatibility with WSFConfig
static NSString * const kKeychainRemapStrategyKey = @"keychainRemapStrategy";
static NSString * const kLoggingEnabledKey = @"loggingEnabled";
static NSString * const kCloudKitStrategyKey = @"cloudKitStrategy";
static NSString * const kICloudContainerRemappingsKey = @"iCloudContainersRemappings";
static NSString * const kAppGroupStrategyKey = @"appGroupStrategy";
static NSString * const kAppGroupRemappingsKey = @"appGroupRemappings";

@implementation WSFConfigLoader

+ (WSFKeychainRemapStrategy)loadKeychainRemapStrategy {
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Only return the stored value if the key exists
    if ([defaults objectForKey:kKeychainRemapStrategyKey]) {
        NSInteger keychainStrategyValue = [defaults integerForKey:kKeychainRemapStrategyKey];
        
        // Validate the strategy value
        if (keychainStrategyValue == WSFKeychainRemapStrategyTeamID || 
            keychainStrategyValue == WSFKeychainRemapStrategyAppGroup) {
            return (WSFKeychainRemapStrategy)keychainStrategyValue;
        }
    }
    
    // Return WSFKeychainRemapStrategyNone if not set or invalid
    return WSFKeychainRemapStrategyNone;
}

+ (WSFCloudKitStrategy)loadCloudKitStrategy {
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Only return the stored value if the key exists
    if ([defaults objectForKey:kCloudKitStrategyKey]) {
        NSInteger cloudKitStrategyValue = [defaults integerForKey:kCloudKitStrategyKey];
        
        // Validate the strategy value against the new enum order
        if (cloudKitStrategyValue == WSFCloudKitStrategyNone ||
            cloudKitStrategyValue == WSFCloudKitStrategyFirstContainer ||
            cloudKitStrategyValue == WSFCloudKitStrategyRemap || 
            cloudKitStrategyValue == WSFCloudKitStrategyNullify) {
            return (WSFCloudKitStrategy)cloudKitStrategyValue;
        }
    }
    
    // Return WSFCloudKitStrategyNone (now 0) if not set or invalid
    return WSFCloudKitStrategyNone;
}

+ (NSDictionary *)loadCloudKitMappings {
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Get iCloud container remappings array from NSUserDefaults
    NSArray *remappingsArray = [defaults objectForKey:kICloudContainerRemappingsKey];
    if (!remappingsArray || ![remappingsArray isKindOfClass:[NSArray class]]) {
        NSLog(@"WarpsignFix [ConfigLoader]: No iCloud container remappings found in NSUserDefaults");
        return @{};
    }
    
    NSMutableDictionary *mappings = [NSMutableDictionary dictionary];
    
    // Convert array of dictionaries to a simple mapping dictionary
    for (NSDictionary *mapping in remappingsArray) {
        if (![mapping isKindOfClass:[NSDictionary class]]) continue;
        
        NSString *original = mapping[@"original"];
        NSString *remapped = mapping[@"remapped"];
        
        if (original && remapped) {
            mappings[original] = remapped;
        }
    }
    
    NSLog(@"WarpsignFix [ConfigLoader]: Loaded %lu CloudKit container remappings", (unsigned long)mappings.count);
    return [mappings copy];
}

+ (BOOL)loadLoggingEnabled {
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Only return the stored value if the key exists
    if ([defaults objectForKey:kLoggingEnabledKey]) {
        return [defaults boolForKey:kLoggingEnabledKey];
    }
    
    // Default value if not set
    return YES;
}

+ (WSFAppGroupStrategy)loadAppGroupStrategy {
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Only return the stored value if the key exists
    if ([defaults objectForKey:kAppGroupStrategyKey]) {
        NSInteger strategyValue = [defaults integerForKey:kAppGroupStrategyKey];
        
        // Validate the strategy value
        if (strategyValue == WSFAppGroupStrategyFirstGroup || 
            strategyValue == WSFAppGroupStrategyFakeGroup ||
            strategyValue == WSFAppGroupStrategyRemap) {
            return (WSFAppGroupStrategy)strategyValue;
        }
    }
    
    // Return WSFAppGroupStrategyNone if not set or invalid
    return WSFAppGroupStrategyNone;
}

+ (NSDictionary *)loadAppGroupMappings {
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Get App Group remappings array from NSUserDefaults
    NSArray *remappingsArray = [defaults objectForKey:kAppGroupRemappingsKey];
    if (!remappingsArray || ![remappingsArray isKindOfClass:[NSArray class]]) {
        NSLog(@"WarpsignFix [ConfigLoader]: No App Group remappings found in NSUserDefaults");
        return @{};
    }
    
    NSMutableDictionary *mappings = [NSMutableDictionary dictionary];
    
    // Convert array of dictionaries to a simple mapping dictionary
    for (NSDictionary *mapping in remappingsArray) {
        if (![mapping isKindOfClass:[NSDictionary class]]) continue;
        
        NSString *original = mapping[@"original"];
        NSString *remapped = mapping[@"remapped"];
        
        if (original && remapped) {
            mappings[original] = remapped;
        }
    }
    
    NSLog(@"WarpsignFix [ConfigLoader]: Loaded %lu App Group remappings", (unsigned long)mappings.count);
    return [mappings copy];
}

@end
