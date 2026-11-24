// WSFKeychainFix.m
// If I feel like it, I should move this away from fishhook.

#import "WSFKeychainFix.h"
#import "WSFConfigLoader.h"
#import <Security/Security.h>
#import <Foundation/Foundation.h>
#import <fishhook.h>
#import "WSFEntitlementUtils.h"

// Declare private LaunchServices interfaces (not publicly available)
@interface LSBundleProxy : NSObject
+ (instancetype)bundleProxyForCurrentProcess;
@property (nonatomic, readonly) NSDictionary *entitlements;
@end

// Function pointer types
typedef OSStatus (*SecItemAddFunc)(CFDictionaryRef, CFTypeRef*);
typedef OSStatus (*SecItemCopyMatchingFunc)(CFDictionaryRef, CFTypeRef*);
typedef OSStatus (*SecItemUpdateFunc)(CFDictionaryRef, CFDictionaryRef);
typedef OSStatus (*SecItemDeleteFunc)(CFDictionaryRef);

// Original function pointers
static SecItemAddFunc original_SecItemAdd = NULL;
static SecItemCopyMatchingFunc original_SecItemCopyMatching = NULL;
static SecItemUpdateFunc original_SecItemUpdate = NULL;
static SecItemDeleteFunc original_SecItemDelete = NULL;

// Current remapping information
static NSString *currentTeamID = nil;
static NSString *firstAppGroup = nil;
static WSFKeychainRemapStrategy keychainRemapStrategy = WSFKeychainRemapStrategyNone; // Default to none
static BOOL loggingEnabled = YES; // Default value

// Forward declarations
static void setupFunctionHooks(void);
static NSString* extractTeamIDFromKeychain(void);

OSStatus custom_SecItemAdd(CFDictionaryRef attributes, CFTypeRef* result);
OSStatus custom_SecItemCopyMatching(CFDictionaryRef query, CFTypeRef* result);
OSStatus custom_SecItemUpdate(CFDictionaryRef attributesToUpdate, CFDictionaryRef attributesToMatch);
OSStatus custom_SecItemDelete(CFDictionaryRef query);

@implementation WSFKeychainFix

+ (void)load {
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        // Load configuration
        [self loadConfiguration];
        
        if (keychainRemapStrategy == WSFKeychainRemapStrategyNone) {
            if (loggingEnabled) {
                NSLog(@"WarpsignFix [Keychain]: No keychain remapping strategy configured, keychain fix disabled");
            }
            return;
        }
        
        // Extract Team ID before hooking Security functions
        currentTeamID = extractTeamIDFromKeychain();
        
        if (!currentTeamID || [currentTeamID length] == 0) {
            if (loggingEnabled) NSLog(@"WarpsignFix [Keychain]: Failed to extract Team ID");
            currentTeamID = @""; // Set to empty string as fallback
        }
        
        // Extract the first app group for potential use
        firstAppGroup = [WSFEntitlementUtils extractFirstAppGroupID];
        
        // Now hook the functions
        setupFunctionHooks();
        
        if (loggingEnabled) {
            NSLog(@"WarpsignFix [Keychain]: Initialized with Team ID: %@, App Group: %@",
                  currentTeamID, firstAppGroup ?: @"None");
            NSLog(@"WarpsignFix [Keychain]: Using strategy: %@",
                  (keychainRemapStrategy == WSFKeychainRemapStrategyTeamID) ? @"Team ID" : @"App Group");
        }
    });
}

static NSString* extractTeamIDFromKeychain(void) {
    NSString *teamID = nil;
    
    // Create a temporary keychain item
    NSDictionary *attributes = @{
        (__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
        (__bridge id)kSecAttrAccount: @"WarpsignFixTemp",
        (__bridge id)kSecAttrService: @"WarpsignFixTempService",
        (__bridge id)kSecValueData: [@"test" dataUsingEncoding:NSUTF8StringEncoding]
    };
    
    OSStatus status = SecItemAdd((__bridge CFDictionaryRef)attributes, NULL);
    
    // Retrieve the item to get its access group
    if (status == errSecSuccess || status == errSecDuplicateItem) {
        NSDictionary *query = @{
            (__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
            (__bridge id)kSecAttrAccount: @"WarpsignFixTemp",
            (__bridge id)kSecAttrService: @"WarpsignFixTempService",
            (__bridge id)kSecReturnAttributes: @YES
        };
        
        CFTypeRef result = NULL;
        status = SecItemCopyMatching((__bridge CFDictionaryRef)query, &result);
        
        if (status == errSecSuccess && result) {
            NSDictionary *attributes = (__bridge_transfer NSDictionary *)result;
            NSString *accessGroup = attributes[(__bridge NSString *)kSecAttrAccessGroup];
            
            if (accessGroup) {
                // Access group format: <teamID>.<rest>
                NSArray *components = [accessGroup componentsSeparatedByString:@"."];
                if (components.count > 0) {
                    teamID = components[0];
                    if (loggingEnabled) NSLog(@"WarpsignFix [Keychain]: Extracted Team ID from keychain: %@", teamID);
                }
            }
        }
        
        // Clean up by deleting the temporary item
        NSDictionary *deleteQuery = @{
            (__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
            (__bridge id)kSecAttrAccount: @"WarpsignFixTemp",
            (__bridge id)kSecAttrService: @"WarpsignFixTempService"
        };
        SecItemDelete((__bridge CFDictionaryRef)deleteQuery);
    }
    
    // Fallback if we couldn't get teamID from keychain
    if (!teamID || [teamID length] == 0) {
        if (loggingEnabled) NSLog(@"WarpsignFix [Keychain]: Falling back to bundle ID extraction");
        NSString *bundleID = [[NSBundle mainBundle] bundleIdentifier];
        NSArray *components = [bundleID componentsSeparatedByString:@"."];
        teamID = components.count > 0 ? components[0] : @"";
    }
    
    return teamID;
}

static NSString* remapAccessGroup(NSString *originalGroup) {
    // If strategy is none, don't modify access groups
    if (keychainRemapStrategy == WSFKeychainRemapStrategyNone) {
        return originalGroup;
    }
    
    // Don't modify special groups like group.* and iCloud.*
    if ([originalGroup hasPrefix:@"group."] || [originalGroup hasPrefix:@"iCloud."]) {
        return originalGroup;
    }
    
    switch (keychainRemapStrategy) {
        case WSFKeychainRemapStrategyNone:
            return originalGroup;
            
        case WSFKeychainRemapStrategyTeamID: {
            // Original team ID-based remapping
            NSArray *components = [originalGroup componentsSeparatedByString:@"."];
            if (components.count > 1) {
                // Replace just the team ID part (first component)
                NSMutableArray *newComponents = [components mutableCopy];
                newComponents[0] = currentTeamID;
                NSString *remappedGroup = [newComponents componentsJoinedByString:@"."];
                if (![remappedGroup isEqualToString:originalGroup] && loggingEnabled) {
                    NSLog(@"WarpsignFix [Keychain]: Remapped group %@ -> %@", originalGroup, remappedGroup);
                }
                return remappedGroup;
            }
            // Fallback to the original behavior if the group doesn't have components
            NSString *remappedGroup = [NSString stringWithFormat:@"%@.%@", currentTeamID, originalGroup];
            if (![remappedGroup isEqualToString:originalGroup] && loggingEnabled) {
                NSLog(@"WarpsignFix [Keychain]: Remapped group %@ -> %@", originalGroup, remappedGroup);
            }
            return remappedGroup;
        }
            
        case WSFKeychainRemapStrategyAppGroup: {
            // New app group-based remapping
            if (firstAppGroup) {
                // App groups are usually in the format "group.com.example.identifier"
                // We want to extract "com.example.identifier" part and use it
                NSString *groupPrefix = @"group.";
                NSString *baseDomain;
                
                if ([firstAppGroup hasPrefix:groupPrefix]) {
                    baseDomain = [firstAppGroup substringFromIndex:[groupPrefix length]];
                } else {
                    baseDomain = firstAppGroup;
                }
                
                // Always ensure group. prefix is present in the remapped access group
                NSString *remappedGroup = [NSString stringWithFormat:@"%@%@", groupPrefix, baseDomain];
                
                if (![remappedGroup isEqualToString:originalGroup] && loggingEnabled) {
                    NSLog(@"WarpsignFix [Keychain]: Remapped group %@ -> %@", originalGroup, remappedGroup);
                }
                return remappedGroup;
            } else {
                if (loggingEnabled) NSLog(@"WarpsignFix [Keychain]: No app group available for remapping, using original group");
                return originalGroup;
            }
        }
    }
    
    return originalGroup; // Fallback
}

static void setupFunctionHooks(void) {
    struct rebinding rebindings[4] = {
        {"SecItemAdd", (void *)custom_SecItemAdd, (void **)&original_SecItemAdd},
        {"SecItemCopyMatching", (void *)custom_SecItemCopyMatching, (void **)&original_SecItemCopyMatching},
        {"SecItemUpdate", (void *)custom_SecItemUpdate, (void **)&original_SecItemUpdate},
        {"SecItemDelete", (void *)custom_SecItemDelete, (void **)&original_SecItemDelete}
    };

    rebind_symbols(rebindings, 4);
}

#pragma mark - Keychain Function Overrides

OSStatus custom_SecItemAdd(CFDictionaryRef attributes, CFTypeRef* result) {
    NSMutableDictionary *modified = [(__bridge NSDictionary *)attributes mutableCopy];
    NSString *originalGroup = modified[(__bridge NSString *)kSecAttrAccessGroup];
    
    if (originalGroup) {
        NSString *remapped = remapAccessGroup(originalGroup);
        // Only modify and log if something actually changed
        if (![remapped isEqualToString:originalGroup]) {
            modified[(__bridge NSString *)kSecAttrAccessGroup] = remapped;
            if (loggingEnabled) {
                NSLog(@"WarpsignFix [Keychain]: Remapped SecItemAdd group %@ -> %@", originalGroup, remapped);
            }
        }
    }
    
    return original_SecItemAdd((__bridge CFDictionaryRef)modified, result);
}

OSStatus custom_SecItemCopyMatching(CFDictionaryRef query, CFTypeRef* result) {
    NSMutableDictionary *modified = [(__bridge NSDictionary *)query mutableCopy];
    NSString *originalGroup = modified[(__bridge NSString *)kSecAttrAccessGroup];
    
    if (originalGroup) {
        NSString *remapped = remapAccessGroup(originalGroup);
        // Only modify and log if something actually changed
        if (![remapped isEqualToString:originalGroup]) {
            modified[(__bridge NSString *)kSecAttrAccessGroup] = remapped;
            if (loggingEnabled) {
                NSLog(@"WarpsignFix [Keychain]: Remapped SecItemCopyMatching group %@ -> %@", originalGroup, remapped);
            }
        }
    }
    
    return original_SecItemCopyMatching((__bridge CFDictionaryRef)modified, result);
}

OSStatus custom_SecItemUpdate(CFDictionaryRef attributesToUpdate, CFDictionaryRef attributesToMatch) {
    NSMutableDictionary *modifiedMatch = [(__bridge NSDictionary *)attributesToMatch mutableCopy];
    NSString *originalGroup = modifiedMatch[(__bridge NSString *)kSecAttrAccessGroup];
    
    if (originalGroup) {
        NSString *remapped = remapAccessGroup(originalGroup);
        // Only modify and log if something actually changed
        if (![remapped isEqualToString:originalGroup]) {
            modifiedMatch[(__bridge NSString *)kSecAttrAccessGroup] = remapped;
            if (loggingEnabled) {
                NSLog(@"WarpsignFix [Keychain]: Remapped SecItemUpdate group %@ -> %@", originalGroup, remapped);
            }
        }
    }
    
    return original_SecItemUpdate(attributesToUpdate, (__bridge CFDictionaryRef)modifiedMatch);
}

OSStatus custom_SecItemDelete(CFDictionaryRef query) {
    NSMutableDictionary *modified = [(__bridge NSDictionary *)query mutableCopy];
    NSString *originalGroup = modified[(__bridge NSString *)kSecAttrAccessGroup];
    
    if (originalGroup) {
        NSString *remapped = remapAccessGroup(originalGroup);
        // Only modify and log if something actually changed
        if (![remapped isEqualToString:originalGroup]) {
            modified[(__bridge NSString *)kSecAttrAccessGroup] = remapped;
            if (loggingEnabled) {
                NSLog(@"WarpsignFix [Keychain]: Remapped SecItemDelete group %@ -> %@", originalGroup, remapped);
            }
        }
    }
    
    return original_SecItemDelete((__bridge CFDictionaryRef)modified);
}

+ (NSString *)currentTeamID {
    return currentTeamID;
}

+ (NSString *)firstAppGroup {
    return firstAppGroup;
}

+ (WSFKeychainRemapStrategy)currentKeychainRemapStrategy {
    return keychainRemapStrategy;
}

+ (BOOL)isLoggingEnabled {
    return loggingEnabled;
}

+ (void)loadConfiguration {
    // Load settings using the minimal loader
    keychainRemapStrategy = [WSFConfigLoader loadKeychainRemapStrategy];
    loggingEnabled = [WSFConfigLoader loadLoggingEnabled];
    
    if (loggingEnabled) {
        NSString *strategyString;
        switch (keychainRemapStrategy) {
            case WSFKeychainRemapStrategyNone:
                strategyString = @"None";
                break;
            case WSFKeychainRemapStrategyTeamID:
                strategyString = @"TeamID";
                break;
            case WSFKeychainRemapStrategyAppGroup:
                strategyString = @"AppGroup";
                break;
            default:
                strategyString = @"Unknown";
                break;
        }
        
        NSLog(@"WarpsignFix [Keychain]: Loaded configuration - strategy=%@, logging=%@", 
              strategyString, loggingEnabled ? @"enabled" : @"disabled");
    }
}

+ (void)checkAppGroupsEntitlement {
    if (!loggingEnabled) return; // Skip if logging disabled
    
    NSLog(@"WarpsignFix [Keychain]: Checking App Groups entitlements");
    
    // Get bundle proxy for current process
    Class bundleProxyClass = NSClassFromString(@"LSBundleProxy");
    if (!bundleProxyClass) {
        NSLog(@"WarpsignFix [Keychain]: LSBundleProxy class not found");
        return;
    }
    
    id proxy = [bundleProxyClass bundleProxyForCurrentProcess];
    if (!proxy) {
        NSLog(@"WarpsignFix [Keychain]: Failed to get bundle proxy");
        return;
    }

    // Get entitlements dictionary
    NSDictionary *entitlements = [proxy entitlements];
    
    if (![entitlements isKindOfClass:[NSDictionary class]]) {
        NSLog(@"WarpsignFix [Keychain]: No valid entitlements found");
        return;
    }

    // Log all entitlements for debugging
    NSLog(@"WarpsignFix [Keychain]: All entitlements: %@", entitlements);

    // Get application groups array
    NSArray *appGroups = entitlements[@"com.apple.security.application-groups"];
    
    if (![appGroups isKindOfClass:[NSArray class]]) {
        NSLog(@"WarpsignFix [Keychain]: No App Groups entitlement found");
        return;
    }

    if (appGroups.count == 0) {
        NSLog(@"WarpsignFix [Keychain]: App Groups entitlement exists but is empty");
        return;
    }

    NSLog(@"WarpsignFix [Keychain]: Found App Groups:");
    for (NSString *groupID in appGroups) {
        NSLog(@"WarpsignFix [Keychain]: - %@", groupID);
    }
    
    // Get container URLs (additional private API)
    if ([proxy respondsToSelector:@selector(groupContainerURLs)]) {
        NSDictionary *containers = [proxy valueForKey:@"groupContainerURLs"];
        NSLog(@"WarpsignFix [Keychain]: Container URLs: %@", containers);
    }
}

@end
