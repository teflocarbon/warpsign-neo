// WSFCloudKitFix.h
#import <Foundation/Foundation.h>
#import "WSFConfigLoader.h" // Import this to get the WSFCloudKitStrategy enum

@interface WSFCloudKitFix : NSObject

// Access to the extracted identifiers
+ (NSString *)firstCloudKitContainer;
+ (NSDictionary *)cloudKitMappings;

// Configuration methods
+ (WSFCloudKitStrategy)currentCloudKitStrategy;
+ (BOOL)isLoggingEnabled;

// Helper methods
+ (void)checkCloudKitEntitlements;

@end
