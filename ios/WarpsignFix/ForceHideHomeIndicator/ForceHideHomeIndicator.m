// ForceHideHomeIndicator: hides the home indicator in the app it's injected
// into (--hide-home-indicator). Written for warpsign-neo to do what Python
// warpsign's prebuilt ForceHideHomeIndicator.dylib does, whose source isn't
// published: when loaded, -[UIViewController prefersHomeIndicatorAutoHidden]
// returns YES, replaced if UIViewController has it and added if not.
//
// Only UIViewController's own method changes, so a view controller that
// overrides it keeps its own answer, as with the original.

#import <UIKit/UIKit.h>
#import <objc/runtime.h>

static BOOL WSFPrefersHomeIndicatorAutoHidden(id self, SEL _cmd) {
    return YES;
}

__attribute__((constructor)) static void WSFForceHideHomeIndicator(void) {
    Class controller = [UIViewController class];
    SEL selector = @selector(prefersHomeIndicatorAutoHidden);
    IMP replacement = (IMP)WSFPrefersHomeIndicatorAutoHidden;
    Method method = class_getInstanceMethod(controller, selector);
    if (method) {
        method_setImplementation(method, replacement);
        NSLog(@"[ForceHideHomeIndicator] Replaced -[UIViewController prefersHomeIndicatorAutoHidden]");
    } else {
        class_addMethod(controller, selector, replacement, "B@:");
        NSLog(@"[ForceHideHomeIndicator] Added -[UIViewController prefersHomeIndicatorAutoHidden]");
    }
}
