#import "WSFEntitlementUtils.h"
#import "WSFConfigLoader.h"

// Private LaunchServices interface
@interface LSBundleProxy : NSObject
+ (instancetype)bundleProxyForCurrentProcess;
@property (nonatomic, readonly) NSDictionary *entitlements;
@end

@implementation WSFEntitlementUtils

// Cache for app group IDs to avoid repeated entitlement lookups
static NSArray<NSString *> *cachedAppGroupIDs = nil;
static NSString *cachedFirstAppGroupID = nil;
static BOOL loggingEnabled = YES;

+ (void)initialize {
    if (self == [WSFEntitlementUtils class]) {
        loggingEnabled = [WSFConfigLoader loadLoggingEnabled];
    }
}

+ (NSDictionary *)getEntitlements {
    Class bundleProxyClass = NSClassFromString(@"LSBundleProxy");
    if (!bundleProxyClass || ![bundleProxyClass respondsToSelector:@selector(bundleProxyForCurrentProcess)]) {
        if (loggingEnabled) NSLog(@"WarpsignFix [Entitlements]: LSBundleProxy class not found");
        return nil;
    }
    
    id proxy = [bundleProxyClass bundleProxyForCurrentProcess];
    if (!proxy || ![proxy respondsToSelector:@selector(entitlements)]) {
        if (loggingEnabled) NSLog(@"WarpsignFix [Entitlements]: Failed to get bundle proxy");
        return nil;
    }

    NSDictionary *entitlements = [proxy entitlements];
    if (![entitlements isKindOfClass:[NSDictionary class]]) {
        if (loggingEnabled) NSLog(@"WarpsignFix [Entitlements]: No valid entitlements found");
        return nil;
    }
    
    return entitlements;
}

+ (NSArray<NSString *> *)extractAppGroupIDs {
    if (cachedAppGroupIDs) {
        return cachedAppGroupIDs;
    }

    if (loggingEnabled) NSLog(@"WarpsignFix [Entitlements]: Extracting App Group IDs from entitlements...");

    NSDictionary *entitlements = [self getEntitlements];
    if (!entitlements) {
        return nil;
    }

    id rawGroups = entitlements[@"com.apple.security.application-groups"];
    if (!rawGroups || ![rawGroups isKindOfClass:[NSArray class]]) {
        if (loggingEnabled) NSLog(@"WarpsignFix [Entitlements]: No App Groups entitlement found or invalid format");
        return nil;
    }

    cachedAppGroupIDs = [(NSArray *)rawGroups copy];
    
    if (loggingEnabled) {
        if (cachedAppGroupIDs.count > 0) {
            NSLog(@"WarpsignFix [Entitlements]: Found %lu App Groups in entitlements", 
                  (unsigned long)cachedAppGroupIDs.count);
        } else {
            NSLog(@"WarpsignFix [Entitlements]: App Groups entitlement is empty");
        }
    }

    return cachedAppGroupIDs;
}

+ (NSString *)extractFirstAppGroupID {
    if (cachedFirstAppGroupID) {
        return cachedFirstAppGroupID;
    }

    NSArray *groupIDs = [self extractAppGroupIDs];
    if (!groupIDs.count) {
        return nil;
    }

    // First try to find a group starting with "group."
    for (NSString *group in groupIDs) {
        if ([group hasPrefix:@"group."]) {
            cachedFirstAppGroupID = group;
            if (loggingEnabled) NSLog(@"WarpsignFix [Entitlements]: Found app group with proper prefix: %@", group);
            return group;
        }
    }

    // Fallback to first group if none start with "group."
    cachedFirstAppGroupID = groupIDs.firstObject;
    if (loggingEnabled) {
        NSLog(@"WarpsignFix [Entitlements]: Warning: Using app group without proper prefix: %@", 
              cachedFirstAppGroupID);
    }
    
    return cachedFirstAppGroupID;
}

+ (BOOL)isAppGroupIDValid:(NSString *)groupID {
    if (!groupID) {
        return NO;
    }
    
    NSArray *validGroups = [self extractAppGroupIDs];
    return [validGroups containsObject:groupID];
}

+ (BOOL)isLoggingEnabled {
    return loggingEnabled;
}

@end 