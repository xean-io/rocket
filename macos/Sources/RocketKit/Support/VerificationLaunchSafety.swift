import Foundation

/// Keeps a verification-marked bundle from falling back to the user's daemon.
public enum VerificationLaunchSafety {
    public static func permitsLaunch(marked: Bool, requestedHome: String?, resolvedHome: String?) -> Bool {
        guard marked else { return true }
        guard let requestedHome, let resolvedHome,
              requestedHome.range(of: #"^/tmp/rkv-[A-Za-z0-9_]{6,24}$"#, options: .regularExpression) != nil else { return false }
        return resolvedHome == requestedHome || resolvedHome == "/private" + requestedHome
    }
}
