// WSFCloudKitFix.m
#import "WSFCloudKitFix.h"
#import "WSFConfigLoader.h"
#import <objc/runtime.h>
#import <CloudKit/CloudKit.h>

// Declare private LaunchServices interfaces (keep this)
@interface LSBundleProxy : NSObject
+ (instancetype)bundleProxyForCurrentProcess;
@property (nonatomic, readonly) NSDictionary *entitlements;
@end

// Current configuration (keep this)
static WSFCloudKitStrategy cloudKitStrategy = WSFCloudKitStrategyNone;
static NSDictionary *cloudKitMappings = nil;
static NSString *firstCloudKitContainerID = nil;
static BOOL loggingEnabled = YES;

// Forward declarations
// static void setupCloudKitHooks(void); // Remove fishhook setup
static NSString* extractFirstCloudKitContainer(void);
static NSString* remapContainerID(NSString *containerID);

// Forward declarations for our custom implementations
static CKContainer* (*Original_CKContainer_containerWithIdentifier)(id, SEL, NSString*);
static id (*Original_CKContainer_initWithIdentifier)(id, SEL, NSString*);

@implementation WSFCloudKitFix

+ (void)load { // Use +load for automatic execution when library loads
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        [self loadConfiguration];

        if (cloudKitStrategy == WSFCloudKitStrategyNone) {
            if (loggingEnabled) {
                NSLog(@"WarpsignFix [CloudKit]: No CloudKit strategy configured, CloudKit fix disabled");
            }
            return;
        }

        firstCloudKitContainerID = extractFirstCloudKitContainer();

        // --- Setup Method Swizzling ---
        [self setupCloudKitSwizzling];
        // ---------------------------

        if (loggingEnabled) {
             NSLog(@"WarpsignFix [CloudKit]: Initialized CloudKit fix with strategy: %@",
                  (cloudKitStrategy == WSFCloudKitStrategyNone) ? @"None" :
                  (cloudKitStrategy == WSFCloudKitStrategyFirstContainer) ? @"First Container" :
                  (cloudKitStrategy == WSFCloudKitStrategyRemap) ? @"Remap" :
                  (cloudKitStrategy == WSFCloudKitStrategyNullify) ? @"Nullify" : @"Unknown");

            if (cloudKitStrategy == WSFCloudKitStrategyRemap) {
                NSLog(@"WarpsignFix [CloudKit]: Loaded %lu CloudKit container mappings", (unsigned long)cloudKitMappings.count);
            } else if (cloudKitStrategy == WSFCloudKitStrategyFirstContainer) {
                NSLog(@"WarpsignFix [CloudKit]: First CloudKit container: %@", firstCloudKitContainerID ?: @"None");
            }
        }
    });
}

// Swizzling setup function
+ (void)setupCloudKitSwizzling {
    Class containerClass = objc_getClass("CKContainer");
    if (!containerClass) {
         if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Error: CKContainer class not found for swizzling.");
         return;
    }

    SEL originalClassSelector = @selector(containerWithIdentifier:);
    SEL swizzledClassSelector = @selector(wsf_containerWithIdentifier:);
    Method originalClassMethod = class_getClassMethod(containerClass, originalClassSelector);
    Method swizzledClassMethod = class_getClassMethod(self, swizzledClassSelector); // Get from *this* class

    if (originalClassMethod && swizzledClassMethod) {
        // Store original implementation before swizzling
        Original_CKContainer_containerWithIdentifier = (CKContainer* (*)(id, SEL, NSString*))method_getImplementation(originalClassMethod);

        method_exchangeImplementations(originalClassMethod, swizzledClassMethod);
        if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Swizzled +[CKContainer containerWithIdentifier:].");
    } else {
         if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Error: Failed to find methods for +[CKContainer containerWithIdentifier:] swizzling.");
    }

    SEL originalInstanceSelector = @selector(initWithIdentifier:);
    SEL swizzledInstanceSelector = @selector(wsf_initWithIdentifier:);
    Method originalInstanceMethod = class_getInstanceMethod(containerClass, originalInstanceSelector);
    Method swizzledInstanceMethod = class_getInstanceMethod(self, swizzledInstanceSelector); // Get from *this* class

    if (originalInstanceMethod && swizzledInstanceMethod) {
         // Store original implementation before swizzling
         Original_CKContainer_initWithIdentifier = (id (*)(id, SEL, NSString*))method_getImplementation(originalInstanceMethod);

        method_exchangeImplementations(originalInstanceMethod, swizzledInstanceMethod);
        if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Swizzled -[CKContainer initWithIdentifier:].");
    } else {
        if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Error: Failed to find methods for -[CKContainer initWithIdentifier:] swizzling.");
    }
}


// Our replacement for +[CKContainer containerWithIdentifier:]
+ (CKContainer *)wsf_containerWithIdentifier:(NSString *)containerIdentifier {
    if (cloudKitStrategy == WSFCloudKitStrategyNullify) {
        if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Nullifying +[CKContainer containerWithIdentifier:%@]", containerIdentifier);
        return nil;
    }

    NSString *remappedID = remapContainerID(containerIdentifier); // Use static helper
    if (loggingEnabled && ![remappedID isEqualToString:containerIdentifier]) {
        NSLog(@"WarpsignFix [CloudKit]: Remapping in +containerWithIdentifier: %@ -> %@", containerIdentifier, remappedID);
    }

    // Call the ORIGINAL implementation using the stored function pointer
    // Note: We call the original implementation directly, not via selector to avoid infinite loop
    if (Original_CKContainer_containerWithIdentifier) {
         return Original_CKContainer_containerWithIdentifier(self, @selector(containerWithIdentifier:), remappedID);
    } else {
         NSLog(@"WarpsignFix [CloudKit]: Error: Original +containerWithIdentifier implementation not found!");
         // Fallback attempt (might cause loop if swizzling failed badly) - less safe
         // return [self wsf_containerWithIdentifier:remappedID];
         return nil; // Safer fallback
    }
}

// Our replacement for -[CKContainer initWithIdentifier:]
- (id)wsf_initWithIdentifier:(NSString *)containerIdentifier {
     if (cloudKitStrategy == WSFCloudKitStrategyNullify) {
        if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Nullifying -[CKContainer initWithIdentifier:%@]", containerIdentifier);
        // Need to decide how to handle nil return from init. Often causes crashes.
        // Maybe return an uninitialized object or handle differently?
        // For now, let's proceed but log heavily.
        NSLog(@"WarpsignFix [CloudKit]: Warning: Nullifying init might lead to unstable behavior.");
        // return nil; // This will likely crash the caller
    }

    NSString *remappedID = remapContainerID(containerIdentifier); // Use static helper
    if (loggingEnabled && ![remappedID isEqualToString:containerIdentifier]) {
        NSLog(@"WarpsignFix [CloudKit]: Remapping in -initWithIdentifier: %@ -> %@", containerIdentifier, remappedID);
    }

    // Call the ORIGINAL implementation using the stored function pointer
    // Note: We call the original implementation directly, not via selector to avoid infinite loop
     if (Original_CKContainer_initWithIdentifier) {
        // 'self' here refers to the CKContainer instance being initialized
        return Original_CKContainer_initWithIdentifier(self, @selector(initWithIdentifier:), remappedID);
     } else {
         NSLog(@"WarpsignFix [CloudKit]: Error: Original -initWithIdentifier implementation not found!");
         // Fallback attempt (might cause loop if swizzling failed badly) - less safe
         // return [self wsf_initWithIdentifier:remappedID];
         return nil; // Safer fallback
     }
}

+ (NSString *)firstCloudKitContainer {
    return firstCloudKitContainerID;
}

+ (NSDictionary *)cloudKitMappings {
    return cloudKitMappings;
}

+ (WSFCloudKitStrategy)currentCloudKitStrategy {
    return cloudKitStrategy;
}

+ (BOOL)isLoggingEnabled {
    return loggingEnabled;
}

+ (void)loadConfiguration {
    // Load settings using the minimal loader
    cloudKitStrategy = [WSFConfigLoader loadCloudKitStrategy];
    loggingEnabled = [WSFConfigLoader loadLoggingEnabled];
    cloudKitMappings = [WSFConfigLoader loadCloudKitMappings];

    if (loggingEnabled) {
        NSString *strategyString;
        switch (cloudKitStrategy) {
            case WSFCloudKitStrategyNone:
                strategyString = @"None";
                break;
            case WSFCloudKitStrategyFirstContainer:
                strategyString = @"First Container";
                break;
            case WSFCloudKitStrategyRemap:
                strategyString = @"Remap";
                break;
            case WSFCloudKitStrategyNullify:
                strategyString = @"Nullify";
                break;
            // Removed default to catch potential issues if new values are added
            // default:
            //     strategyString = @"Unknown";
            //     break;
        }

        NSLog(@"WarpsignFix [CloudKit]: Loaded CloudKit configuration - strategy=%@, logging=%@",
              strategyString, loggingEnabled ? @"enabled" : @"disabled");

        if (cloudKitStrategy == WSFCloudKitStrategyRemap) {
            NSLog(@"WarpsignFix [CloudKit]: Loaded %lu iCloud container remappings",
                  (unsigned long)cloudKitMappings.count);

            NSUInteger count = 0;
            for (NSString *key in cloudKitMappings) {
                if (count < 3) {
                    NSLog(@"WarpsignFix [CloudKit]: Mapping %@ -> %@", key, cloudKitMappings[key]);
                    count++;
                } else {
                    NSLog(@"WarpsignFix [CloudKit]: ... and %lu more mappings",
                          (unsigned long)cloudKitMappings.count - 3);
                    break;
                }
            }
        }
    }
}

+ (void)checkCloudKitEntitlements {
    if (!loggingEnabled) return;

    NSLog(@"WarpsignFix [CloudKit]: Checking CloudKit entitlements");

    NSDictionary *entitlements = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"Entitlements"];
    if (!entitlements) {
        Class bundleProxyClass = NSClassFromString(@"LSBundleProxy");
        if (bundleProxyClass && [bundleProxyClass respondsToSelector:@selector(bundleProxyForCurrentProcess)]) {
            id proxy = [bundleProxyClass bundleProxyForCurrentProcess];
            if ([proxy respondsToSelector:@selector(entitlements)]) {
                entitlements = [proxy entitlements];
            }
        }

        if (!entitlements) {
            NSLog(@"WarpsignFix [CloudKit]: No entitlements found");
            return;
        }
    }

    NSLog(@"WarpsignFix [CloudKit]: All entitlements: %@", entitlements);

    NSArray *containers =
        entitlements[@"com.apple.developer.icloud-container-identifiers"] ?:
        entitlements[@"com.apple.developer.ubiquity-container-identifiers"] ?:
        entitlements[@"com.apple.developer.ubiquity-kvstore-identifier"];

    if (![containers isKindOfClass:[NSArray class]]) {
        NSLog(@"WarpsignFix [CloudKit]: No CloudKit container identifiers found");
        return;
    }

    if (containers.count == 0) {
        NSLog(@"WarpsignFix [CloudKit]: CloudKit container identifiers exist but array is empty");
        return;
    }

    NSLog(@"WarpsignFix [CloudKit]: Found CloudKit containers:");
    for (NSString *containerID in containers) {
        NSLog(@"WarpsignFix [CloudKit]: - %@", containerID);
    }
}

@end

#pragma mark - CloudKit Helper Functions (Keep these static functions)

static NSString* extractFirstCloudKitContainer(void) {
     if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Extracting CloudKit containers");

    NSDictionary *entitlements = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"Entitlements"];
    if (!entitlements) {
        Class bundleProxyClass = NSClassFromString(@"LSBundleProxy");
        if (bundleProxyClass && [bundleProxyClass respondsToSelector:@selector(bundleProxyForCurrentProcess)]) {
            id proxy = [bundleProxyClass bundleProxyForCurrentProcess];
            if ([proxy respondsToSelector:@selector(entitlements)]) {
                entitlements = [proxy entitlements];
            }
        }

        if (!entitlements) {
            if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: No entitlements found");
            return nil;
        }
    }

    NSArray *containers =
        entitlements[@"com.apple.developer.icloud-container-identifiers"] ?:
        entitlements[@"com.apple.developer.ubiquity-container-identifiers"] ?:
        entitlements[@"com.apple.developer.ubiquity-kvstore-identifier"];

    if (![containers isKindOfClass:[NSArray class]] || containers.count == 0) {
        if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: No CloudKit containers found");
        return nil;
    }

    NSString *container = [containers firstObject];
    if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: First CloudKit container: %@", container);
    return container;
}

static NSString* remapContainerID(NSString *containerID) {
    if (!containerID) return containerID;

    NSString *remappedContainer = nil;

    if (cloudKitStrategy == WSFCloudKitStrategyFirstContainer && firstCloudKitContainerID) {
        remappedContainer = firstCloudKitContainerID;
    }
    else if (cloudKitStrategy == WSFCloudKitStrategyRemap && cloudKitMappings && cloudKitMappings[containerID]) {
        remappedContainer = cloudKitMappings[containerID];
        if (loggingEnabled) {
            NSLog(@"WarpsignFix [CloudKit]: Found mapping for container %@: %@", containerID, remappedContainer);
        }
    }

    // --- No logging needed here, done in the calling swizzled method ---
    // if (remappedContainer && ![remappedContainer isEqualToString:containerID]) {
    //     if (loggingEnabled) NSLog(@"WarpsignFix [CloudKit]: Remapping CloudKit container: %@ -> %@", containerID, remappedContainer);
    //     return remappedContainer;
    // }
    // return containerID;

    // Return remapped ID if found, otherwise original
    return remappedContainer ?: containerID;
}
