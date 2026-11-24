// WSFAppGroupFix.m
#import "WSFAppGroupFix.h"
#import "WSFConfigLoader.h"
#import <objc/runtime.h>
#import <Foundation/Foundation.h>
#import "WSFEntitlementUtils.h"

// Private interfaces if needed (LSBundleProxy example)
@interface LSBundleProxy : NSObject
+ (instancetype)bundleProxyForCurrentProcess;
@property (nonatomic, readonly) NSDictionary *entitlements;
@property (nonatomic, readonly) NSArray<NSString *> *groupContainerIdentifiers; // Alternative way? Check headers
@end

// --- Original Method IMP Pointers ---
static NSURL* (*Original_FileManager_containerURLForSecurityApplicationGroupIdentifier)(id, SEL, NSString*);
static id (*Original_UserDefaults_initWithSuiteName)(id, SEL, NSString*);

// --- Static Configuration ---
static WSFAppGroupStrategy appGroupStrategy = WSFAppGroupStrategyNone;
static NSDictionary *appGroupMappings = nil; // << NEW: Store mappings
static NSString *firstAppGroupID = nil;
static NSArray *validAppGroupIDs = nil; // << NEW: Store all valid groups from entitlements
static BOOL loggingEnabled = YES;

// --- Forward Declarations ---

@implementation WSFAppGroupFix

+ (void)load {
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        // Load fundamental config first
        loggingEnabled = [WSFConfigLoader loadLoggingEnabled];
        appGroupStrategy = [WSFConfigLoader loadAppGroupStrategy];

        if (appGroupStrategy == WSFAppGroupStrategyNone) {
            if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: No App Group strategy configured, fix disabled.");
            return;
        }

        // Load mappings BEFORE extracting local info (though order might not matter much here)
        if (appGroupStrategy == WSFAppGroupStrategyRemap) {
             appGroupMappings = [WSFConfigLoader loadAppGroupMappings];
             if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Loaded %lu specific App Group mappings.", (unsigned long)appGroupMappings.count);
        }

        // Extract local entitlement info (needed for FirstGroup, FakeGroup, and validation)
        firstAppGroupID = [WSFEntitlementUtils extractFirstAppGroupID];
        validAppGroupIDs = [WSFEntitlementUtils extractAppGroupIDs];

        // Setup swizzling
        [self setupAppGroupSwizzling];

        if (loggingEnabled) {
             NSString *strategyDesc;
             switch(appGroupStrategy) {
                 case WSFAppGroupStrategyFirstGroup: strategyDesc = @"First Group"; break;
                 case WSFAppGroupStrategyFakeGroup: strategyDesc = @"Fake Group"; break;
                 case WSFAppGroupStrategyRemap: strategyDesc = @"Remap (using specific mappings)"; break;
                 default: strategyDesc = @"None/Unknown"; break; // Should have returned earlier if None
             }
             NSLog(@"WarpsignFix [AppGroup]: Initialized App Group fix. Strategy: %@. First Valid Group: %@. Total Valid Groups: %lu",
                   strategyDesc, firstAppGroupID ?: @"None", (unsigned long)validAppGroupIDs.count);
        }
    });
}

#pragma mark - Public Configuration Access

+ (WSFAppGroupStrategy)currentAppGroupStrategy {
    return appGroupStrategy;
}

+ (NSDictionary *)appGroupMappings {
    return appGroupMappings;
}

+ (void)setupAppGroupSwizzling {
    Class fileManagerClass = [NSFileManager class];
    SEL fmOriginalSel = @selector(containerURLForSecurityApplicationGroupIdentifier:);
    SEL fmSwizzledSel = @selector(wsf_containerURLForSecurityApplicationGroupIdentifier:);
    Method fmOriginalMethod = class_getInstanceMethod(fileManagerClass, fmOriginalSel);
    Method fmSwizzledMethod = class_getInstanceMethod(self, fmSwizzledSel); // Swizzle NSFileManager instance method

    if (fmOriginalMethod && fmSwizzledMethod) {
        Original_FileManager_containerURLForSecurityApplicationGroupIdentifier = (NSURL* (*)(id, SEL, NSString*))method_getImplementation(fmOriginalMethod);
        method_exchangeImplementations(fmOriginalMethod, fmSwizzledMethod);
        if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Swizzled -[NSFileManager containerURLForSecurityApplicationGroupIdentifier:].");
    } else {
         if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Error: Failed to swizzle NSFileManager containerURL method.");
    }

    Class userDefaultsClass = [NSUserDefaults class];
    SEL udOriginalSel = @selector(initWithSuiteName:);
    SEL udSwizzledSel = @selector(wsf_initWithSuiteName:);
    Method udOriginalMethod = class_getInstanceMethod(userDefaultsClass, udOriginalSel);
    Method udSwizzledMethod = class_getInstanceMethod(self, udSwizzledSel);

    if (udOriginalMethod && udSwizzledMethod) {
         Original_UserDefaults_initWithSuiteName = (id (*)(id, SEL, NSString*))method_getImplementation(udOriginalMethod);
        method_exchangeImplementations(udOriginalMethod, udSwizzledMethod);
        if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Swizzled -[NSUserDefaults initWithSuiteName:].");
    } else {
         if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Error: Failed to swizzle NSUserDefaults initWithSuiteName method.");
    }
}

// Swizzled NSFileManager method
- (NSURL *)wsf_containerURLForSecurityApplicationGroupIdentifier:(NSString *)originalGroupID {
    if (!originalGroupID) {
         // Pass through if nil ID is given
        return Original_FileManager_containerURLForSecurityApplicationGroupIdentifier ? Original_FileManager_containerURLForSecurityApplicationGroupIdentifier(self, @selector(containerURLForSecurityApplicationGroupIdentifier:), originalGroupID) : nil;
    }

    // Use the utility function directly
    BOOL isValid = [WSFEntitlementUtils isAppGroupIDValid:originalGroupID];
    NSString *remappedID = originalGroupID;
    BOOL wasRemapped = NO;

    // Apply strategy logic
    switch (appGroupStrategy) {
        case WSFAppGroupStrategyRemap: {
            NSString *mappedID = appGroupMappings[originalGroupID];
            if (mappedID) {
                remappedID = mappedID;
                wasRemapped = YES;
                 if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] Remap(Map): containerURL for '%@' -> '%@'", originalGroupID, remappedID);
            } else if (!isValid && firstAppGroupID) {
                 // Fallback to FirstGroup ONLY if original is invalid and no specific mapping found? (Optional)
                 // remappedID = firstAppGroupID;
                 // wasRemapped = YES;
                 // if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] Remap(Fallback): containerURL for invalid '%@' -> '%@'", originalGroupID, remappedID);
                 if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] Remap(Map): No specific mapping for '%@', using original.", originalGroupID);
            }
            // If mappedID is nil and isValid is YES, remappedID remains originalGroupID
            break;
        }
        case WSFAppGroupStrategyFirstGroup: {
             // Use first group only if the requested one is invalid according to entitlements
            if (!isValid && firstAppGroupID) {
                remappedID = firstAppGroupID;
                wasRemapped = YES;
                 if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] FirstGroup: containerURL for invalid '%@' -> '%@'", originalGroupID, remappedID);
            }
            break;
        }
        case WSFAppGroupStrategyFakeGroup: {
            if (!isValid) {
                NSArray *paths = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES);
                NSString *documentsDirectory = [paths firstObject];
                NSURL *docURL = [NSURL fileURLWithPath:documentsDirectory isDirectory:YES];

                // Ensure Documents exists (though it almost always should)
                NSError *error = nil;
                [[NSFileManager defaultManager] createDirectoryAtURL:docURL withIntermediateDirectories:YES attributes:nil error:&error];
                // Log error?

                if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] FakeGroup: containerURL for invalid '%@' -> Documents (%@)", originalGroupID, docURL.path);
                return docURL; // Return fake path immediately
            }
            // If valid, proceed to call original with originalGroupID
            break;
        }
        case WSFAppGroupStrategyNone:
            // Do nothing, use originalGroupID
            break;
    }

    // Call original implementation with potentially remapped ID
    if (!Original_FileManager_containerURLForSecurityApplicationGroupIdentifier) {
         if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Error: Original containerURL IMP not found!");
        return nil; // Or handle error appropriately
    }
    return Original_FileManager_containerURLForSecurityApplicationGroupIdentifier(self, @selector(containerURLForSecurityApplicationGroupIdentifier:), remappedID);
}


// Swizzled NSUserDefaults method
- (NSUserDefaults *)wsf_initWithSuiteName:(NSString *)originalSuiteName {
     // Handle nil suite name - call original
     if (!originalSuiteName) {
         return Original_UserDefaults_initWithSuiteName ? Original_UserDefaults_initWithSuiteName(self, @selector(initWithSuiteName:), originalSuiteName) : nil;
     }

    // Only apply logic if it looks like an app group suite name
    BOOL looksLikeAppGroup = [originalSuiteName hasPrefix:@"group."];
    // Use the utility function directly
    BOOL isValid = [WSFEntitlementUtils isAppGroupIDValid:originalSuiteName];
    NSString *remappedSuiteName = originalSuiteName;
    BOOL wasRemapped = NO;

    if (looksLikeAppGroup) {
         switch (appGroupStrategy) {
             case WSFAppGroupStrategyRemap: {
                 NSString *mappedID = appGroupMappings[originalSuiteName];
                 if (mappedID) {
                     remappedSuiteName = mappedID;
                     wasRemapped = YES;
                     if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] Remap(Map): initWithSuiteName for '%@' -> '%@'", originalSuiteName, remappedSuiteName);
                 } else if (!isValid && firstAppGroupID) {
                     // Optional fallback? Same as containerURL
                     // remappedSuiteName = firstAppGroupID;
                     // wasRemapped = YES;
                     // if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] Remap(Fallback): initWithSuiteName for invalid '%@' -> '%@'", originalSuiteName, remappedSuiteName);
                     if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] Remap(Map): No specific mapping for suite '%@', using original.", originalSuiteName);
                 }
                 break;
             }
             case WSFAppGroupStrategyFirstGroup: {
                 if (!isValid && firstAppGroupID) {
                     remappedSuiteName = firstAppGroupID;
                     wasRemapped = YES;
                     if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] FirstGroup: initWithSuiteName for invalid '%@' -> '%@'", originalSuiteName, remappedSuiteName);
                 }
                 break;
             }
             case WSFAppGroupStrategyFakeGroup: {
                 if (!isValid) {
                     if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup] FakeGroup: initWithSuiteName for invalid group '%@' -> standardUserDefaults", originalSuiteName);
                     // Returning standardUserDefaults *instead* of calling original init
                     // Note: This returns a singleton, not a new instance like init usually does.
                     // This might have implications, but is likely safer than returning nil from init.
                     return [NSUserDefaults standardUserDefaults];
                 }
                 break;
             }
             case WSFAppGroupStrategyNone:
                 // Do nothing
                 break;
         }
    } // else it doesn't look like an app group, so we ignore it

    // Call original implementation with potentially remapped suite name
    if (!Original_UserDefaults_initWithSuiteName) {
        if (loggingEnabled) NSLog(@"WarpsignFix [AppGroup]: Error: Original initWithSuiteName IMP not found!");
        return nil; // Init returning nil can be problematic
    }
    return Original_UserDefaults_initWithSuiteName(self, @selector(initWithSuiteName:), remappedSuiteName);
}

+ (BOOL)isLoggingEnabled {
    return loggingEnabled;
}

+ (NSString *)firstAppGroupID {
    // Already extracted and stored during load
    return firstAppGroupID;
}

+ (NSArray<NSString *> *)validAppGroupIDs {
    // Already extracted and stored during load
    return validAppGroupIDs;
}

+ (void)checkAppGroupEntitlements {
    if (![self isLoggingEnabled]) return; // Use the class method

    NSLog(@"WarpsignFix [AppGroup]: Checking App Group entitlements...");
    NSArray<NSString *> *groups = [WSFEntitlementUtils extractAppGroupIDs]; // Use utility

    if (!groups) {
        NSLog(@"WarpsignFix [AppGroup]: Failed to retrieve App Group entitlements or none found.");
    } else if (groups.count == 0) {
        NSLog(@"WarpsignFix [AppGroup]: App Groups entitlement exists but is empty.");
    } else {
        NSLog(@"WarpsignFix [AppGroup]: Found %lu App Group(s):", (unsigned long)groups.count);
        for (NSString *groupID in groups) {
            NSLog(@"WarpsignFix [AppGroup]: - %@", groupID);
        }
        NSLog(@"WarpsignFix [AppGroup]: First App Group detected by utility: %@", [WSFEntitlementUtils extractFirstAppGroupID] ?: @"None");
    }
}

@end
