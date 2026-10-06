import Foundation
import RocketKit

/// Runs before any controller is created, including LaunchServices relaunches.
enum VerificationRuntime {
    static var isMarked: Bool { Bundle.main.object(forInfoDictionaryKey: "RocketVerificationBundle") as? Bool == true }
    static var windowTitle: String {
        isMarked ? Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Rocket Verify" : "Rocket"
    }
    static func title(for section: String) -> String { isMarked ? "\(windowTitle) — \(section)" : section }
    static var appearance: String? {
        let args = CommandLine.arguments
        guard let index = args.firstIndex(of: "--verification-appearance"), args.indices.contains(index + 1),
              ["light", "dark"].contains(args[index + 1]) else { return nil }
        return args[index + 1]
    }

    static func permitsLaunch() -> Bool {
        guard isMarked else { return true }
        let rawHome = ProcessInfo.processInfo.environment["ROCKET_HOME"]
        let url = rawHome.map { URL(fileURLWithPath: $0) }
        let attrs = url.flatMap { try? FileManager.default.attributesOfItem(atPath: $0.path) }
        let isPrivate = (attrs?[.posixPermissions] as? NSNumber)?.intValue == 0o700 &&
            (attrs?[.ownerAccountID] as? NSNumber)?.uint32Value == getuid() &&
            attrs?[.type] as? FileAttributeType == .typeDirectory
        guard isPrivate, VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: rawHome,
                                                               resolvedHome: url?.resolvingSymlinksInPath().path) else {
            FileHandle.standardError.write(Data("Rocket verification requires an explicit owned 0700 /tmp/rkv-* home.\n".utf8))
            return false
        }
        return true
    }

    /// Public evidence only: no daemon token or inherited environment is written.
    @MainActor
    static func recordReady(_ controller: RocketController) {
        guard isMarked, controller.eventsReceived > 0, case .connected(let health) = controller.connection else { return }
        let report: [String: Any] = ["app_pid": Int(getpid()), "home": controller.home.path,
                                     "daemon_pid": health.pid, "sse_events": controller.eventsReceived,
                                     "projects": controller.store.projectNames]
        let url = Bundle.main.bundleURL.deletingLastPathComponent().appendingPathComponent("app-ready.json")
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]) {
            try? data.write(to: url, options: .atomic)
        }
    }
}
