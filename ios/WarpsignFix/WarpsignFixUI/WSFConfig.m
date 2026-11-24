// WSFConfig.m
#import "WSFConfig.h"
#import "WSFExternalConfig.h" // Import for Plist path access

// NSUserDefaults keys
static NSString * const kHasLaunchedBeforeKey = @"WarpsignFixHasLaunchedBefore";
static NSString * const kKeychainRemapStrategyKey = @"keychainRemapStrategy";
static NSString * const kLoggingEnabledKey = @"loggingEnabled";
static NSString * const kCloudKitStrategyKey = @"cloudKitStrategy";
static NSString * const kICloudContainerRemappingsKey = @"iCloudContainersRemappings";
static NSString * const kAppGroupStrategyKey = @"appGroupStrategy";
static NSString * const kAppGroupRemappingsKey = @"appGroupRemappings";
static NSString * const kDebugRemappingOverrideKey = @"debugRemappingOverride"; // New key

@implementation WSFConfig

#pragma mark - Singleton

+ (instancetype)sharedConfig {
    static WSFConfig *sharedInstance = nil;
    static dispatch_once_t onceToken;
    
    dispatch_once(&onceToken, ^{
        sharedInstance = [[self alloc] init];
        [sharedInstance loadRemappingsFromBundle];
        [sharedInstance loadConfig];
    });
    
    return sharedInstance;
}

+ (instancetype)shared {
    return [self sharedConfig];
}

#pragma mark - Initialization

- (instancetype)init {
    self = [super init];
    if (self) {
        // Default values if no config exists
        _keychainRemapStrategy = WSFKeychainRemapStrategyNone; // Default to None (do nothing)
        _cloudKitStrategy = WSFCloudKitStrategyNone; // Default to None (now 0)
        _appGroupStrategy = WSFAppGroupStrategyNone;
        _loggingEnabled = YES;
        _iCloudContainerRemappings = nil;
        _appGroupRemappings = nil;
        _remappingsFileLoadedSuccessfully = NO; // Default to NO until loaded
        _debugRemappingOverride = WSFDebugRemappingOverrideDefault; // Default to Default (0)
    }
    return self;
}

#pragma mark - Loading and Saving

- (void)loadRemappingsFromBundle {
    _remappingsFileLoadedSuccessfully = NO; // Reset flag before attempting load
    
    // Find the JSON file in the main app bundle
    NSString *jsonPath = [[NSBundle mainBundle] pathForResource:@"remappings" ofType:@"json"];
    
    if (!jsonPath) {
        NSLog(@"WarpsignFixUI: remappings.json not found in bundle");
        // Keep _remappingsFileLoadedSuccessfully = NO;
        return;
    }
    
    NSError *error;
    NSData *jsonData = [NSData dataWithContentsOfFile:jsonPath options:0 error:&error];
    
    if (error) {
        NSLog(@"WarpsignFixUI: Error reading remappings.json: %@", error.localizedDescription);
        // Keep _remappingsFileLoadedSuccessfully = NO;
        return;
    }
    
    NSDictionary *remappingsDict = [NSJSONSerialization JSONObjectWithData:jsonData 
                                                                 options:0 
                                                                   error:&error];
    
    if (error) {
        NSLog(@"WarpsignFixUI: Error parsing remappings.json: %@", error.localizedDescription);
        // Keep _remappingsFileLoadedSuccessfully = NO;
        return;
    }

    // If we reached here, the file was loaded and parsed successfully
    _remappingsFileLoadedSuccessfully = YES;
    NSLog(@"WarpsignFixUI: Successfully loaded and parsed remappings.json");

    // Extract iCloud container remappings
    NSArray *iCloudContainers = remappingsDict[@"iCloudContainers"];
    if (iCloudContainers && [iCloudContainers isKindOfClass:[NSArray class]]) {
        self.iCloudContainerRemappings = iCloudContainers;
        
        // Save to NSUserDefaults
        NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
        [defaults setObject:iCloudContainers forKey:kICloudContainerRemappingsKey];
        [defaults synchronize];
        
        NSLog(@"WarpsignFixUI: Loaded %lu iCloud container remappings", 
              (unsigned long)iCloudContainers.count);
    } else {
        self.iCloudContainerRemappings = nil; // Ensure it's nil if key missing or wrong type
        NSLog(@"WarpsignFixUI: No iCloud container remappings found or invalid format in JSON");
    }
    
    // Extract App Group remappings
    NSArray *appGroups = remappingsDict[@"appGroups"];
    if (appGroups && [appGroups isKindOfClass:[NSArray class]]) {
        self.appGroupRemappings = appGroups;
        
        // Save to NSUserDefaults
        NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
        [defaults setObject:appGroups forKey:kAppGroupRemappingsKey];
        [defaults synchronize];
        
        NSLog(@"WarpsignFixUI: Loaded %lu App Group remappings", 
              (unsigned long)appGroups.count);
    } else {
         self.appGroupRemappings = nil; // Ensure it's nil if key missing or wrong type
         NSLog(@"WarpsignFixUI: No App Group remappings found or invalid format in JSON");
    }
}

- (void)loadConfig {
    // Using standard NSUserDefaults works for communication between app and its extensions
    // without requiring App Groups entitlement
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // Load Debug Override first
    if ([defaults objectForKey:kDebugRemappingOverrideKey]) {
         NSInteger overrideValue = [defaults integerForKey:kDebugRemappingOverrideKey];
         // Validate the loaded value against the enum range
         if (overrideValue >= WSFDebugRemappingOverrideDefault && overrideValue <= WSFDebugRemappingOverrideForceUnavailable) {
            self.debugRemappingOverride = (WSFDebugRemappingOverride)overrideValue;
         } else {
            self.debugRemappingOverride = WSFDebugRemappingOverrideDefault; // Fallback to default if invalid
            NSLog(@"WarpsignFixUI: WARNING - Invalid value %ld found for %@, resetting to Default.", (long)overrideValue, kDebugRemappingOverrideKey);
         }
     } else {
         self.debugRemappingOverride = WSFDebugRemappingOverrideDefault; // Ensure default if key doesn't exist
     }

    // Try to load existing values
    if ([defaults objectForKey:kKeychainRemapStrategyKey]) {
        NSInteger keychainStrategyValue = [defaults integerForKey:kKeychainRemapStrategyKey];
        if (keychainStrategyValue >= 0 && keychainStrategyValue <= 2) {
            self.keychainRemapStrategy = (WSFKeychainRemapStrategy)keychainStrategyValue;
        }
    }
    
    // Only set if the key exists to avoid overriding default
    if ([defaults objectForKey:kLoggingEnabledKey]) {
        self.loggingEnabled = [defaults boolForKey:kLoggingEnabledKey];
    }
    
    // Load CloudKit strategy if it exists
    if ([defaults objectForKey:kCloudKitStrategyKey]) {
        NSInteger cloudKitValue = [defaults integerForKey:kCloudKitStrategyKey];
        // Validate value range against new enum order
        if (cloudKitValue >= WSFCloudKitStrategyNone && cloudKitValue <= WSFCloudKitStrategyNullify) { 
            self.cloudKitStrategy = (WSFCloudKitStrategy)cloudKitValue;
        }
    }
    
    // Load iCloud Container remappings if they exist (these are now loaded from JSON first, but keep defaults load as fallback/persistence)
    if ([defaults objectForKey:kICloudContainerRemappingsKey]) {
        // Only load from defaults if JSON loading failed? Or always override?
        // Let's keep loading from defaults for persistence, JSON load primarily populates it initially or on update.
        if (!self.remappingsFileLoadedSuccessfully) { // Only load from defaults if JSON load failed
             self.iCloudContainerRemappings = [defaults objectForKey:kICloudContainerRemappingsKey];
             NSLog(@"WarpsignFixUI: Loaded iCloud remappings from UserDefaults as JSON failed.");
        }
    }
    
    // Load App Group strategy if it exists
    if ([defaults objectForKey:kAppGroupStrategyKey]) {
        NSInteger appGroupValue = [defaults integerForKey:kAppGroupStrategyKey];
        if (appGroupValue >= 0 && appGroupValue <= 3) {
            self.appGroupStrategy = (WSFAppGroupStrategy)appGroupValue;
        }
    }
    
    // Load App Group remappings if they exist
    if ([defaults objectForKey:kAppGroupRemappingsKey]) {
        if (!self.remappingsFileLoadedSuccessfully) { // Only load from defaults if JSON load failed
             self.appGroupRemappings = [defaults objectForKey:kAppGroupRemappingsKey];
             NSLog(@"WarpsignFixUI: Loaded App Group remappings from UserDefaults as JSON failed.");
        }
    }
    
    NSLog(@"WarpsignFixUI: Loaded config from NSUserDefaults: keychain=%@, cloudkit=%@, appgroup=%@, logging=%@, debugRemapOverride=%ld",
          [self keychainStrategyName],
          [self cloudKitStrategyName],
          [self appGroupStrategyName],
          self.loggingEnabled ? @"enabled" : @"disabled",
          (long)self.debugRemappingOverride); // Log new override value
}

- (NSString *)keychainStrategyName {
    switch (self.keychainRemapStrategy) {
        case WSFKeychainRemapStrategyNone:
            return @"None";
        case WSFKeychainRemapStrategyTeamID:
            return @"TeamID";
        case WSFKeychainRemapStrategyAppGroup:
            return @"AppGroup";
        default:
            return @"Unknown";
    }
}

- (NSString *)cloudKitStrategyName {
    switch (self.cloudKitStrategy) {
        case WSFCloudKitStrategyNone:
            return @"None";
        case WSFCloudKitStrategyFirstContainer:
            return @"First Container";
        case WSFCloudKitStrategyRemap:
            return @"Remap";
        case WSFCloudKitStrategyNullify:
            return @"Nullify";
        default:
            return @"Unknown";
    }
}

- (NSString *)appGroupStrategyName {
    switch (self.appGroupStrategy) {
        case WSFAppGroupStrategyNone:
            return @"None";
        case WSFAppGroupStrategyFirstGroup:
            return @"First Group";
        case WSFAppGroupStrategyFakeGroup:
            return @"Fake Group";
        case WSFAppGroupStrategyRemap:
            return @"Remap";
        default:
            return @"Unknown";
    }
}

- (BOOL)saveConfig {
    // NSUserDefaults automatically shares settings between app and its extensions
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    
    // --- Save current values to NSUserDefaults --- 
    [defaults setInteger:self.keychainRemapStrategy forKey:kKeychainRemapStrategyKey];
    
    [defaults setBool:self.loggingEnabled forKey:kLoggingEnabledKey];
    
    // Save CloudKit strategy
    [defaults setInteger:self.cloudKitStrategy forKey:kCloudKitStrategyKey];
    
    // Save iCloud Container remappings (originally from JSON, persisted via defaults)
    if (self.iCloudContainerRemappings) {
        [defaults setObject:self.iCloudContainerRemappings forKey:kICloudContainerRemappingsKey];
    } else {
        [defaults removeObjectForKey:kICloudContainerRemappingsKey];
    }
    
    // Save App Group strategy
    [defaults setInteger:self.appGroupStrategy forKey:kAppGroupStrategyKey];
    
    // Save App Group remappings (originally from JSON, persisted via defaults)
    if (self.appGroupRemappings) {
        [defaults setObject:self.appGroupRemappings forKey:kAppGroupRemappingsKey];
    } else {
        [defaults removeObjectForKey:kAppGroupRemappingsKey];
    }

    // Save Debug Override
    [defaults setInteger:self.debugRemappingOverride forKey:kDebugRemappingOverrideKey];
    
    // Make sure NSUserDefaults values are synchronized to disk
    BOOL successUserDefaults = [defaults synchronize];
    
    if (successUserDefaults) {
        NSLog(@"WarpsignFixUI: Config saved successfully to NSUserDefaults");

        // --- Now, attempt to save relevant settings back to external Plist --- 
        NSString *configPath = [WSFExternalConfig configPlistPath];
        if (!configPath) {
             NSLog(@"WarpsignFixUI: ERROR - Could not get external config plist path to write back. Plist not updated.");
             // Still return success based on UserDefaults save
             return successUserDefaults; 
        }

        // Create dictionary with ONLY the keys managed by the external plist
        NSDictionary *externalConfigDict = @{
             kLoggingEnabledKey: @(self.loggingEnabled),
             kKeychainRemapStrategyKey: @(self.keychainRemapStrategy),
             kCloudKitStrategyKey: @(self.cloudKitStrategy),
             kAppGroupStrategyKey: @(self.appGroupStrategy)
        };

        NSError *writeError = nil;
        NSData *plistData = [NSPropertyListSerialization dataWithPropertyList:externalConfigDict
                                                                         format:NSPropertyListXMLFormat_v1_0
                                                                        options:0
                                                                          error:&writeError];
        
        if (plistData && !writeError) {
            if ([plistData writeToFile:configPath atomically:YES]) {
                NSLog(@"WarpsignFixUI: Successfully updated external config plist at %@", configPath);
            } else {
                NSLog(@"WarpsignFixUI: ERROR - Failed to write updated config to external plist at %@.", configPath);
                // Note: We don't change the overall success state here, as UserDefaults succeeded.
            }
        } else {
            NSLog(@"WarpsignFixUI: ERROR - Failed to serialize config dictionary for external plist: %@", writeError.localizedDescription);
        }
        // --- End external Plist update ---

    } else {
        NSLog(@"WarpsignFixUI: Error saving config to NSUserDefaults");
    }
    
    return successUserDefaults; // Return success based on NSUserDefaults synchronization
}

@end
