// WarpsignFix.m
#import "WarpsignFix.h"
#import "WSFKeychainFix.h"
#import "WSFCloudKitFix.h"
#import "WSFConfigLoader.h"
#import "WSFAppGroupFix.h"
#import <Foundation/Foundation.h>

@implementation WarpsignFix

+ (void)load {
    NSLog(@"Hello from WarpsignFix!");
}

+ (WSFKeychainRemapStrategy)currentKeychainRemapStrategy {
    return [WSFKeychainFix currentKeychainRemapStrategy];
}

+ (WSFCloudKitStrategy)currentCloudKitStrategy {
    return [WSFCloudKitFix currentCloudKitStrategy];
}

+ (NSDictionary *)cloudKitMappings {
    return [WSFCloudKitFix cloudKitMappings];
}

+ (WSFAppGroupStrategy)currentAppGroupStrategy {
    return [WSFAppGroupFix currentAppGroupStrategy];
}

+ (NSDictionary *)appGroupMappings {
    return [WSFAppGroupFix appGroupMappings];
}

+ (BOOL)isLoggingEnabled {
    return [WSFConfigLoader loadLoggingEnabled];
}

+ (void)loadConfiguration {
    // This will load configuration from NSUserDefaults
    // WSFKeychainFix and WSFCloudKitFix will load their specific 
    // configurations during initialization
    BOOL loggingEnabled = [WSFConfigLoader loadLoggingEnabled];
    
    // Display current configuration if logging is enabled
    if (loggingEnabled) {
        WSFKeychainRemapStrategy keychainStrategy = [WSFConfigLoader loadKeychainRemapStrategy];
        WSFCloudKitStrategy cloudKitStrategy = [WSFConfigLoader loadCloudKitStrategy];
        WSFAppGroupStrategy appGroupStrategy = [WSFConfigLoader loadAppGroupStrategy];
        
        NSString *keychainStrategyString;
        switch (keychainStrategy) {
            case WSFKeychainRemapStrategyNone:
                keychainStrategyString = @"None";
                break;
            case WSFKeychainRemapStrategyTeamID:
                keychainStrategyString = @"TeamID";
                break;
            case WSFKeychainRemapStrategyAppGroup:
                keychainStrategyString = @"AppGroup";
                break;
            default:
                keychainStrategyString = @"Unknown";
                break;
        }
        
        NSString *cloudKitStrategyString;
        switch (cloudKitStrategy) {
            case WSFCloudKitStrategyRemap:
                cloudKitStrategyString = @"Remap";
                break;
            case WSFCloudKitStrategyNullify:
                cloudKitStrategyString = @"Nullify";
                break;
            case WSFCloudKitStrategyNone:
                cloudKitStrategyString = @"None";
                break;
            case WSFCloudKitStrategyFirstContainer:
                cloudKitStrategyString = @"First Container";
                break;
            default:
                cloudKitStrategyString = @"Unknown";
                break;
        }
        
        NSString *appGroupStrategyString;
        switch (appGroupStrategy) {
            case WSFAppGroupStrategyNone:
                appGroupStrategyString = @"None";
                break;
            case WSFAppGroupStrategyFirstGroup:
                appGroupStrategyString = @"First Group";
                break;
            case WSFAppGroupStrategyFakeGroup:
                appGroupStrategyString = @"Fake Group";
                break;
            case WSFAppGroupStrategyRemap:
                appGroupStrategyString = @"Remap";
                break;
            default:
                appGroupStrategyString = @"Unknown";
                break;
        }
        
        NSLog(@"WarpsignFix: Loaded configuration - keychain=%@, cloudkit=%@, appgroup=%@, logging=%@",
              keychainStrategyString, cloudKitStrategyString, appGroupStrategyString, 
              loggingEnabled ? @"enabled" : @"disabled");
        
        // Check for iCloud container remappings
        NSDictionary *cloudKitMappings = [WSFConfigLoader loadCloudKitMappings];
        if (cloudKitMappings.count > 0) {
            NSLog(@"WarpsignFix: Loaded %lu iCloud container remappings", (unsigned long)cloudKitMappings.count);
        }
        
        NSDictionary *appGroupMappings = [WSFConfigLoader loadAppGroupMappings];
        if (appGroupMappings.count > 0) {
            NSLog(@"WarpsignFix: Loaded %lu App Group remappings", (unsigned long)appGroupMappings.count);
        }
    }
}

+ (void)checkAppGroupsEntitlement {
    [WSFKeychainFix checkAppGroupsEntitlement];
}

+ (void)checkCloudKitEntitlements {
    [WSFCloudKitFix checkCloudKitEntitlements];
}

@end
