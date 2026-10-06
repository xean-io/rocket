import Observation
import RocketKit
import SwiftUI

enum SidebarItem: Hashable, Codable {
    case project(String)
    case ports
    case jobs
    case owners
}

/// One confirmation flow shared by new deployments and historical reruns.
struct DeploymentIntent: Identifiable {
    let project: String
    let env: String
    let rerunning: Job?
    var id: String { "\(project)/\(env)/\(rerunning?.id ?? "new")" }
}

/// Window-level navigation shared with menu commands and the menu bar extra.
@Observable
@MainActor
final class AppNavigation {
    var selection: SidebarItem?
    var selectedService: ServiceKey?
    var selectedJob: Job.ID?
    var showInspector = false
    var pendingDeployment: DeploymentIntent?
    /// Env picked in the project toolbar, per project (nil = project default).
    var envs: [String: String] = [:]

    init() {
        // Debug/verification hook: `Rocket -RocketInitialSection ports|jobs|owners`.
        switch UserDefaults.standard.string(forKey: "RocketInitialSection") {
        case "ports": selection = .ports
        case "jobs": selection = .jobs
        case "owners": selection = .owners
        default: break
        }
        if let key = UserDefaults.standard.string(forKey: "RocketSelectService"), let slash = key.firstIndex(of: "/") {
            let service = ServiceKey(project: String(key[..<slash]), service: String(key[key.index(after: slash)...]))
            selection = .project(service.project)
            selectedService = service
            showInspector = true
        }
    }

    var selectedProject: String? {
        if case .project(let name) = selection { return name }
        return nil
    }

    func env(for project: String, in store: RocketStore) -> String? {
        store.validatedEnv(envs[project], for: project)
    }

    func reconcileEnvs(in store: RocketStore) {
        envs = store.validatedEnvironmentSelections(envs)
    }

    func showProjects(in store: RocketStore) {
        if selectedProject == nil {
            selection = store.projectNames.first.map(SidebarItem.project)
        }
    }

    func showLogs(for key: ServiceKey) {
        selection = .project(key.project)
        selectedService = key
        showInspector = true
    }

    func showJob(_ job: Job) {
        selection = .jobs
        selectedService = nil
        selectedJob = job.id
        showInspector = true
    }

    func confirmDeployment(project: String, env: String, rerunning job: Job? = nil) {
        pendingDeployment = DeploymentIntent(project: project, env: env, rerunning: job)
    }
}
