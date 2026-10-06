import AppKit
import RocketKit
import SwiftUI

@main
enum Main {
    static func main() {
        guard VerificationRuntime.permitsLaunch() else { exit(1) }
        if CommandLine.arguments.contains("--self-check") {
            SelfCheck.run()
        } else {
            RocketApp.main()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        if VerificationRuntime.isMarked, let appearance = VerificationRuntime.appearance {
            NSApp.appearance = NSAppearance(named: appearance == "light" ? .aqua : .darkAqua)
        }
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    // The menu bar extra keeps Rocket useful after the window closes.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

struct RocketApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var controller = RocketController()
    @State private var navigation = AppNavigation()

    var body: some Scene {
        WindowGroup(VerificationRuntime.windowTitle, id: "main") {
            RootView(controller: controller, navigation: navigation)
                .frame(minWidth: 860, minHeight: 520)
                .tint(.xeanViolet)
                .task {
                    if controller.connection == .idle { await controller.connect() }
                }
                .task(id: controller.eventsReceived) { VerificationRuntime.recordReady(controller) }
        }
        .defaultSize(width: 1180, height: 740)
        .commands { RocketCommands(controller: controller, navigation: navigation) }

        MenuBarExtra {
            MenuBarContent(controller: controller, navigation: navigation)
                .tint(.xeanViolet)
        } label: {
            MenuBarLabel(running: controller.store.runningCount, connected: controller.connection.isConnected)
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsView(controller: controller)
                .tint(.xeanViolet)
        }
    }
}

/// Status item label: rocket glyph plus the running count.
struct MenuBarLabel: View {
    let running: Int
    let connected: Bool

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: connected ? "airplane.departure" : "airplane")
            if running > 0 { Text("\(running)").monospacedDigit() }
        }
        .accessibilityLabel(connected ? "\(VerificationRuntime.windowTitle), \(running) services running"
                                     : "\(VerificationRuntime.windowTitle), disconnected")
    }
}
