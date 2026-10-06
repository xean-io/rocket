import RocketKit
import SwiftUI

/// Menu commands with keyboard shortcuts. Actions target the selected service,
/// or the whole selected project when no service is selected.
struct RocketCommands: Commands {
    let controller: RocketController
    let navigation: AppNavigation

    private var project: String? { navigation.selectedService?.project ?? navigation.selectedProject }
    private var services: [String]? { navigation.selectedService.map { [$0.service] } }

    var body: some Commands {
        CommandMenu("Services") {
            Button("Up") {
                guard let project else { return }
                Task { await controller.up(project: project, services: services, env: navigation.env(for: project, in: controller.store)) }
            }
            .keyboardShortcut("u", modifiers: [.command, .shift])
            .disabled(project == nil)

            Button("Restart") {
                guard let project else { return }
                Task { await controller.restart(project: project, services: services, env: navigation.env(for: project, in: controller.store)) }
            }
            .keyboardShortcut("r")
            .disabled(project == nil)

            Button("Stop") {
                guard let project else { return }
                Task { await controller.down(project: project, services: services) }
            }
            .keyboardShortcut(".")
            .disabled(project == nil)

            Divider()

            Button(navigation.showInspector ? "Hide Logs" : "Show Logs") {
                navigation.showInspector.toggle()
            }
            .keyboardShortcut("l")

            Button("Refresh") {
                Task { await controller.refreshAll() }
            }
            .keyboardShortcut("r", modifiers: [.command, .shift])

            Divider()

            Button("Collect Garbage") {
                Task { await controller.gc() }
            }
        }

        CommandGroup(after: .sidebar) {
            Button("Projects") { navigation.showProjects(in: controller.store) }
                .keyboardShortcut("1")
            Button("Ports") { navigation.selection = .ports }
                .keyboardShortcut("2")
            Button("Jobs") { navigation.selection = .jobs }
                .keyboardShortcut("3")
            Button("Owners") { navigation.selection = .owners }
                .keyboardShortcut("4")
            Divider()
        }
    }
}
