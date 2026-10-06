import Testing
@testable import RocketKit

@Suite("Verification launch isolation")
struct VerificationSafetyTests {
    @Test func normalBundleKeepsItsNormalHomeBehavior() {
        #expect(VerificationLaunchSafety.permitsLaunch(marked: false, requestedHome: nil, resolvedHome: nil))
    }

    @Test(arguments: [nil, "", "/Users/me/.rocket", "/tmp/rocket", "/tmp/rkv-a", "/tmp/rkv-abc123/subdir",
                      "/tmp/rkv-abc123/../real", "relative/rkv-abc123", "/private/tmp/rkv-abc123"] as [String?])
    func verificationRequiresExplicitShortPrivateHome(home: String?) {
        #expect(!VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: home, resolvedHome: home))
    }

    @Test func acceptsCanonicalSystemTmpAndRejectsEscapingSymlinks() {
        #expect(VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: "/tmp/rkv-abc123",
                                                      resolvedHome: "/private/tmp/rkv-abc123"))
        #expect(VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: "/tmp/rkv-abc123",
                                                      resolvedHome: "/tmp/rkv-abc123"))
        #expect(!VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: "/tmp/rkv-abc123",
                                                       resolvedHome: "/Users/me/.rocket"))
        #expect(!VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: "/tmp/rkv-abc123",
                                                       resolvedHome: "/private/tmp/rkv-other1"))
        #expect(!VerificationLaunchSafety.permitsLaunch(marked: true, requestedHome: "/tmp/rkv-abc123", resolvedHome: nil))
    }
}
