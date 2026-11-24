#import "WSFExternalConfig.h"
#import "WSFConfig.h" // For enum definitions and keys
#import "WSFReadmeText.h" // Import the header for the README constant

// NSUserDefaults keys (should match WSFConfig.m)
static NSString * const kLoggingEnabledKey = @"loggingEnabled";
static NSString * const kKeychainRemapStrategyKey = @"keychainRemapStrategy";
static NSString * const kCloudKitStrategyKey = @"cloudKitStrategy";
static NSString * const kAppGroupStrategyKey = @"appGroupStrategy";
// Add other keys if they need to be overridden by the plist

static NSString * const kConfigPlistName = @"warpsignfix_config.plist";

@implementation WSFExternalConfig

// Helper to get the path to the Documents directory
+ (NSString *)documentsDirectoryPath {
    NSArray<NSString *> *paths = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES);
    return paths.firstObject;
}

// Helper to get the full path to the configuration plist in Documents
+ (NSString *)configPlistPath {
    NSString *documentsPath = [self documentsDirectoryPath];
    if (!documentsPath) {
        NSLog(@"[WarpsignFixExternalConfig] ERROR: Could not find Documents directory.");
        return nil;
    }
    return [documentsPath stringByAppendingPathComponent:kConfigPlistName];
}

// Defines the default structure and values for the config plist
+ (NSDictionary *)defaultConfigurationDictionary {
    return @{
        kLoggingEnabledKey: @(YES), // Default logging to YES
        kKeychainRemapStrategyKey: @(WSFKeychainRemapStrategyTeamID), // Default to 1 (TeamID)
        kCloudKitStrategyKey: @(WSFCloudKitStrategyNullify),       // Default to 3 (Nullify)
        kAppGroupStrategyKey: @(WSFAppGroupStrategyFakeGroup)        // Default to 2 (Fake Group)
    };
}

// Ensures the config file exists (creating it from defaults if necessary)
// and returns the loaded configuration dictionary.
+ (nullable NSDictionary *)ensureAndLoadConfigFile {
    NSString *configPath = [self configPlistPath];
    if (!configPath) return nil; // Error finding path

    NSFileManager *fileManager = [NSFileManager defaultManager];
    NSDictionary *configDict = nil;

    if (![fileManager fileExistsAtPath:configPath]) {
        NSLog(@"[WarpsignFixExternalConfig] Config file not found at %@. Creating with defaults.", configPath);
        NSDictionary *defaultsDict = [self defaultConfigurationDictionary];
        NSError *writeError = nil;

        // Use PropertyListSerialization for robust Plist writing
        NSData *plistData = [NSPropertyListSerialization dataWithPropertyList:defaultsDict
                                                                         format:NSPropertyListXMLFormat_v1_0
                                                                        options:0
                                                                          error:&writeError];

        if (plistData && !writeError) {
            if ([plistData writeToFile:configPath atomically:YES]) {
                NSLog(@"[WarpsignFixExternalConfig] Successfully created default config file at %@.", configPath);
                configDict = defaultsDict; // Use the defaults we just wrote

                // --- Attempt to write a README file alongside the default config ---
                NSString *readmePath = [[configPath stringByDeletingLastPathComponent] stringByAppendingPathComponent:@"warpsignfix_config_README.txt"];
                // Use the constant string from WSFReadmeText.h
                NSString *readmeContent = WSFReadmeContent;
                NSError *readmeError = nil;
                if ([readmeContent writeToFile:readmePath atomically:YES encoding:NSUTF8StringEncoding error:&readmeError]) {
                     NSLog(@"[WarpsignFixExternalConfig] Successfully created README file at %@.", readmePath);
                } else {
                     NSLog(@"[WarpsignFixExternalConfig] WARNING: Failed to write README file at %@: %@", readmePath, readmeError.localizedDescription);
                }
                // --- End README write ---

            } else {
                NSLog(@"[WarpsignFixExternalConfig] ERROR: Failed to write default config file to %@.", configPath);
                // Proceed without external config if writing failed
            }
        } else {
             NSLog(@"[WarpsignFixExternalConfig] ERROR: Failed to serialize default config dictionary: %@", writeError.localizedDescription);
             // Proceed without external config
        }

    } else {
         NSLog(@"[WarpsignFixExternalConfig] Config file found at %@. Loading existing.", configPath);
         // File exists, try to load it
         configDict = [NSDictionary dictionaryWithContentsOfFile:configPath];
         if (!configDict) {
             NSLog(@"[WarpsignFixExternalConfig] WARNING: Existing config file at %@ could not be read or parsed. Check its format. Proceeding without external overrides.", configPath);
             // If reading fails, we proceed without external config
         }
    }

    // Return the loaded dictionary (or nil if creation/loading failed)
    return configDict;
}

+ (void)loadAndUpdateUserDefaults {
    NSLog(@"[WarpsignFixExternalConfig] Starting external config load...");

    NSDictionary *configDict = [self ensureAndLoadConfigFile];

    if (!configDict) {
        NSLog(@"[WarpsignFixExternalConfig] Could not load or create external config file. Aborting external config update.");
        return;
    }

    NSLog(@"[WarpsignFixExternalConfig] Successfully loaded config dictionary. Applying to UserDefaults...");

    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    BOOL changesMade = NO;

    // --- Update UserDefaults based on plist values --- 

    // Logging Enabled (BOOL)
    id loggingValue = configDict[kLoggingEnabledKey];
    if (loggingValue && [loggingValue isKindOfClass:[NSNumber class]]) {
        BOOL plistValue = [loggingValue boolValue];
        // Check against current NSUserDefaults value BEFORE setting
        if ([defaults boolForKey:kLoggingEnabledKey] != plistValue) {
             NSLog(@"[WarpsignFixExternalConfig] Updating '%@' from config: %d", kLoggingEnabledKey, plistValue);
             [defaults setBool:plistValue forKey:kLoggingEnabledKey];
             changesMade = YES;
        }
    } else if (loggingValue) { // Value exists but is wrong type
         NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid type for '%@' in config. Expected BOOL (Number). Ignored.", kLoggingEnabledKey);
    }

    // Keychain Remap Strategy (Enum/Integer)
    id keychainValue = configDict[kKeychainRemapStrategyKey];
    if (keychainValue && [keychainValue isKindOfClass:[NSNumber class]]) {
        NSInteger plistValue = [keychainValue integerValue];
        if (plistValue >= WSFKeychainRemapStrategyNone && plistValue <= WSFKeychainRemapStrategyAppGroup) {
            // Check against current NSUserDefaults value BEFORE setting
            if ([defaults integerForKey:kKeychainRemapStrategyKey] != plistValue) {
                 NSLog(@"[WarpsignFixExternalConfig] Updating '%@' from config: %ld", kKeychainRemapStrategyKey, (long)plistValue);
                 [defaults setInteger:plistValue forKey:kKeychainRemapStrategyKey];
                 changesMade = YES;
            }
        } else {
            NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid value %ld for '%@' in config. Allowed range [0-2]. Ignored.", (long)plistValue, kKeychainRemapStrategyKey);
        }
    } else if (keychainValue) {
         NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid type for '%@' in config. Expected Integer (Number). Ignored.", kKeychainRemapStrategyKey);
    }

    // CloudKit Strategy (Enum/Integer)
    id cloudkitValue = configDict[kCloudKitStrategyKey];
    if (cloudkitValue && [cloudkitValue isKindOfClass:[NSNumber class]]) {
        NSInteger plistValue = [cloudkitValue integerValue];
        if (plistValue >= WSFCloudKitStrategyNone && plistValue <= WSFCloudKitStrategyNullify) {
             // Check against current NSUserDefaults value BEFORE setting
             if ([defaults integerForKey:kCloudKitStrategyKey] != plistValue) {
                 NSLog(@"[WarpsignFixExternalConfig] Updating '%@' from config: %ld", kCloudKitStrategyKey, (long)plistValue);
                 [defaults setInteger:plistValue forKey:kCloudKitStrategyKey];
                 changesMade = YES;
            }
        } else {
             NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid value %ld for '%@' in config. Allowed range [0-3]. Ignored.", (long)plistValue, kCloudKitStrategyKey);
        }
    } else if (cloudkitValue) {
         NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid type for '%@' in config. Expected Integer (Number). Ignored.", kCloudKitStrategyKey);
    }

    // App Group Strategy (Enum/Integer)
    id appgroupValue = configDict[kAppGroupStrategyKey];
    if (appgroupValue && [appgroupValue isKindOfClass:[NSNumber class]]) {
        NSInteger plistValue = [appgroupValue integerValue];
        if (plistValue >= WSFAppGroupStrategyNone && plistValue <= WSFAppGroupStrategyRemap) {
            // Check against current NSUserDefaults value BEFORE setting
            if ([defaults integerForKey:kAppGroupStrategyKey] != plistValue) {
                 NSLog(@"[WarpsignFixExternalConfig] Updating '%@' from config: %ld", kAppGroupStrategyKey, (long)plistValue);
                 [defaults setInteger:plistValue forKey:kAppGroupStrategyKey];
                 changesMade = YES;
            }
        } else {
             NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid value %ld for '%@' in config. Allowed range [0-3]. Ignored.", (long)plistValue, kAppGroupStrategyKey);
        }
    } else if (appgroupValue) {
        NSLog(@"[WarpsignFixExternalConfig] WARNING: Invalid type for '%@' in config. Expected Integer (Number). Ignored.", kAppGroupStrategyKey);
    }

    // --- End UserDefaults updates ---

    // Synchronize if changes were made
    if (changesMade) {
        BOOL success = [defaults synchronize];
        NSLog(@"[WarpsignFixExternalConfig] Synchronized UserDefaults after applying config changes. Success: %d", success);
    } else {
        NSLog(@"[WarpsignFixExternalConfig] No changes from config needed for UserDefaults.");
    }

    NSLog(@"[WarpsignFixExternalConfig] Finished external config load.");
}

@end 