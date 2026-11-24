import Foundation
import ObjectiveC

/// Spike (docs/spikes/anisette-on-device.md): can a development-signed app
/// get anisette data from iOS's own AuthKit? Asks the system the way Xcode
/// and AltServer do on a Mac and reports only *whether* each piece came back
/// and how long it is. Never shows, logs or stores the values: they identify
/// this device to Apple.
///
/// Read-only: it never provisions, erases or syncs anisette, and sends
/// nothing over the network itself (the request it builds headers for is
/// never sent).
enum AnisetteProbe {
    struct Finding: Identifiable {
        let id = UUID()
        let step: String
        let detail: String
        /// nil: informational
        let ok: Bool?
    }

    /// Headers GrandSlam (gsa.apple.com) and developerservices2 expect.
    static let anisetteHeaders = [
        "X-Apple-I-MD", "X-Apple-I-MD-M", "X-Apple-I-MD-RINFO",
        "X-Apple-I-MD-LU", "X-Mme-Device-Id", "X-Apple-I-SRL-NO",
        "X-Mme-Client-Info", "X-Apple-I-Client-Time",
    ]

    private static let authKitPath = "/System/Library/PrivateFrameworks/AuthKit.framework/AuthKit"
    private static let coreADIPath = "/System/Library/PrivateFrameworks/CoreADI.framework/CoreADI"
    private static let timeout: TimeInterval = 15

    static func run() async -> [Finding] {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                continuation.resume(returning: runBlocking())
            }
        }
    }

    private static func runBlocking() -> [Finding] {
        var findings: [Finding] = []

        guard dlopen(authKitPath, RTLD_NOW) != nil else {
            findings.append(.init(step: "Load AuthKit", detail: "dlopen failed", ok: false))
            return findings
        }
        findings.append(.init(step: "Load AuthKit", detail: "loaded", ok: true))
        let coreADI = dlopen(coreADIPath, RTLD_NOW) != nil
        findings.append(.init(step: "Load CoreADI", detail: coreADI ? "loaded (not called)" : "dlopen failed", ok: nil))

        findings.append(contentsOf: probeDevice())
        findings.append(probeProvisioningController())
        findings.append(contentsOf: probeAppleIDSession())
        return findings
    }

    // MARK: AKDevice

    /// Local properties; tells us whether AuthKit's classes work in-process
    /// at all, apart from akd.
    private static func probeDevice() -> [Finding] {
        guard let cls = NSClassFromString("AKDevice") as? NSObject.Type else {
            return [.init(step: "AKDevice", detail: "class missing", ok: false)]
        }
        let current = NSSelectorFromString("currentDevice")
        guard cls.responds(to: current),
              let device = cls.perform(current)?.takeUnretainedValue() as? NSObject
        else {
            return [.init(step: "AKDevice.currentDevice", detail: "unavailable", ok: false)]
        }
        let keys = ["serverFriendlyDescription", "uniqueDeviceIdentifier", "serialNumber", "localUserUUID"]
        let present = keys.filter { key in
            guard device.responds(to: NSSelectorFromString(key)) else { return false }
            return describedLength(device.value(forKey: key)) != nil
        }
        return [.init(
            step: "AKDevice.currentDevice",
            detail: present.isEmpty ? "no properties returned" : "returned " + present.joined(separator: ", "),
            ok: !present.isEmpty
        )]
    }

    // MARK: AKAnisetteProvisioningController

    private typealias AnisetteCompletion = @convention(block) (AnyObject?, NSError?) -> Void
    private typealias AnisetteFn = @convention(c) (AnyObject, Selector, AnisetteCompletion) -> Void

    /// -[AKAnisetteProvisioningController anisetteDataWithCompletion:] asks
    /// akd for the current machine ID and one-time password.
    private static func probeProvisioningController() -> Finding {
        let step = "AKAnisetteProvisioningController"
        guard let cls = NSClassFromString("AKAnisetteProvisioningController") as? NSObject.Type else {
            return .init(step: step, detail: "class missing", ok: false)
        }
        let controller = cls.init()
        let sel = NSSelectorFromString("anisetteDataWithCompletion:")
        guard controller.responds(to: sel),
              let imp = class_getMethodImplementation(object_getClass(controller), sel)
        else {
            return .init(step: step, detail: "anisetteDataWithCompletion: missing", ok: false)
        }
        let call = unsafeBitCast(imp, to: AnisetteFn.self)

        let done = DispatchSemaphore(value: 0)
        var result: Finding?
        let lock = NSLock()
        call(controller, sel) { data, error in
            let finding: Finding
            if let error {
                finding = .init(step: step, detail: "error " + describe(error), ok: false)
            } else if let data = data as? NSObject {
                let keys = ["machineID", "oneTimePassword", "routingInfo"]
                let parts = keys.map { key -> String in
                    guard data.responds(to: NSSelectorFromString(key)) else { return "\(key): no selector" }
                    if let length = describedLength(data.value(forKey: key)) { return "\(key): \(length)" }
                    return "\(key): empty"
                }
                let gotOTP = parts.contains { $0.hasPrefix("oneTimePassword: ") && !$0.hasSuffix("empty") && !$0.hasSuffix("no selector") }
                finding = .init(step: step, detail: parts.joined(separator: "; "), ok: gotOTP)
            } else {
                finding = .init(step: step, detail: "no data and no error", ok: false)
            }
            lock.lock(); result = finding; lock.unlock()
            done.signal()
        }
        if done.wait(timeout: .now() + timeout) == .timedOut {
            return .init(step: step, detail: "no answer after \(Int(timeout)) s", ok: false)
        }
        lock.lock(); defer { lock.unlock() }
        return result ?? .init(step: step, detail: "no result", ok: false)
    }

    // MARK: AKAppleIDSession

    private typealias InitFn = @convention(c) (AnyObject, Selector, NSString) -> UnsafeMutableRawPointer?
    private typealias HeadersFn = @convention(c) (AnyObject, Selector, NSURLRequest) -> UnsafeMutableRawPointer?

    /// -[AKAppleIDSession appleIDHeadersForRequest:] is what AltServer's Mail
    /// plugin calls on macOS to get the whole anisette header set.
    private static func probeAppleIDSession() -> [Finding] {
        let step = "AKAppleIDSession headers"
        guard let cls = NSClassFromString("AKAppleIDSession") as? NSObject.Type else {
            return [.init(step: step, detail: "class missing", ok: false)]
        }
        let initSel = NSSelectorFromString("initWithIdentifier:")
        let headersSel = NSSelectorFromString("appleIDHeadersForRequest:")
        guard let allocated = cls.perform(NSSelectorFromString("alloc"))?.takeUnretainedValue(),
              allocated.responds(to: initSel),
              let initImp = class_getMethodImplementation(object_getClass(allocated), initSel)
        else {
            return [.init(step: step, detail: "initWithIdentifier: missing", ok: false)]
        }
        // init consumes alloc's +1 and returns +1 (ns_returns_retained), so
        // the result is taken retained; Swift's own retain of `allocated`
        // stays balanced.
        let initFn = unsafeBitCast(initImp, to: InitFn.self)
        guard let raw = initFn(allocated, initSel, "com.apple.gs.xcode.auth" as NSString) else {
            return [.init(step: step, detail: "initWithIdentifier: returned nil", ok: false)]
        }
        let session = Unmanaged<AnyObject>.fromOpaque(raw).takeRetainedValue()
        guard session.responds(to: headersSel),
              let headersImp = class_getMethodImplementation(object_getClass(session), headersSel)
        else {
            return [.init(step: step, detail: "appleIDHeadersForRequest: missing", ok: false)]
        }
        let headersFn = unsafeBitCast(headersImp, to: HeadersFn.self)
        // Never sent: only used to ask which headers AuthKit would add.
        let request = NSURLRequest(url: URL(string: "https://gsa.apple.com/grandslam/GsService2")!)

        let done = DispatchSemaphore(value: 0)
        var headers: [String: Any]?
        let lock = NSLock()
        DispatchQueue.global(qos: .userInitiated).async {
            // Autoreleased (+0) result
            var value: [String: Any]?
            if let raw = headersFn(session, headersSel, request) {
                value = Unmanaged<NSDictionary>.fromOpaque(raw).takeUnretainedValue() as? [String: Any]
            }
            lock.lock(); headers = value; lock.unlock()
            done.signal()
        }
        if done.wait(timeout: .now() + timeout) == .timedOut {
            return [.init(step: step, detail: "no answer after \(Int(timeout)) s", ok: false)]
        }
        lock.lock(); let got = headers ?? [:]; lock.unlock()
        if got.isEmpty {
            return [.init(step: step, detail: "returned no headers", ok: false)]
        }

        var findings: [Finding] = []
        let lowered = Dictionary(got.map { ($0.key.lowercased(), $0.value) }, uniquingKeysWith: { a, _ in a })
        for name in anisetteHeaders {
            if let length = describedLength(lowered[name.lowercased()]) {
                findings.append(.init(step: name, detail: "present, \(length)", ok: true))
            } else {
                findings.append(.init(step: name, detail: "absent", ok: false))
            }
        }
        let known = Set(anisetteHeaders.map { $0.lowercased() })
        let others = got.keys.filter { !known.contains($0.lowercased()) }.sorted()
        if !others.isEmpty {
            // Header names only, never values
            findings.append(.init(step: "Other headers", detail: others.joined(separator: ", "), ok: nil))
        }
        return findings
    }

    // MARK: Helpers

    /// "N chars"/"N bytes"/"number" for a non-empty value, nil otherwise.
    /// Deliberately never the value.
    private static func describedLength(_ value: Any?) -> String? {
        switch value {
        case let string as String: return string.isEmpty ? nil : "\(string.count) chars"
        case let data as Data: return data.isEmpty ? nil : "\(data.count) bytes"
        case let number as NSNumber: return number.uint64Value == 0 ? nil : "number"
        case is UUID, is NSUUID: return "UUID"
        case .some: return "object"
        case .none: return nil
        }
    }

    /// Domain and code only: descriptions can quote account details.
    private static func describe(_ error: NSError) -> String {
        var text = "\(error.domain) \(error.code)"
        if let underlying = error.userInfo[NSUnderlyingErrorKey] as? NSError {
            text += " ← \(underlying.domain) \(underlying.code)"
        }
        return text
    }
}
