import RocketKit
import SwiftUI

/// Root container: sidebar + detail, connection states, error alert.
struct RootView: View {
    @Bindable var controller: RocketController
    @Bindable var navigation: AppNavigation
    @State private var addingProject = false

    /// Observe every selected project's options, regardless of the open pane.
    private var selectedEnvironmentOptions: [String: [String]] {
        Dictionary(uniqueKeysWithValues: navigation.envs.keys.map {
            ($0, controller.store.knownEnvs(for: $0))
        })
    }

    var body: some View {
        NavigationSplitView {
            SidebarView(
                projects: controller.store.projectNames.map { name in
                    SidebarProject(name: name, path: controller.store.path(of: name),
                                   active: controller.store.activeCount(in: name),
                                   total: controller.store.runs(in: name).count)
                },
                jobsAvailable: controller.store.jobsAvailable,
                ownerCount: controller.store.ownerGroups.count,
                connected: controller.connection.isConnected,
                selection: $navigation.selection,
                onAdd: { addingProject = true },
                onRefresh: { Task { await controller.refreshAll() } },
                onRemove: { name in Task { await controller.removeProject(name) } }
            )
            .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 320)
        } detail: {
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(XeanBackdrop())
        }
        .toolbar {
            ToolbarItem(placement: .navigation) {
                ConnectionBadge(state: controller.connection, live: controller.streamLive)
            }
            ToolbarItem(placement: .primaryAction) {
                Button("Add Project", systemImage: "plus") { addingProject = true }
                    .keyboardShortcut("o")
                    .help("Register a directory that contains rocket.yaml (⌘O)")
                    .disabled(!controller.connection.isConnected)
            }
        }
        .fileImporter(isPresented: $addingProject, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result {
                Task { await controller.addProject(path: url.path) }
            }
        }
        .alert("Rocket", isPresented: Binding(get: { controller.lastError != nil },
                                              set: { if !$0 { controller.lastError = nil } })) {
            Button("OK", role: .cancel) { controller.lastError = nil }
        } message: {
            Text(controller.lastError ?? "")
        }
        .sheet(item: $navigation.pendingDeployment) { intent in
            DeploymentConfirmationView(project: intent.project, env: intent.env,
                                       isRerun: intent.rerunning != nil,
                                       onCancel: { navigation.pendingDeployment = nil },
                                       onConfirm: {
                navigation.pendingDeployment = nil
                Task {
                    let job: Job?
                    if let previous = intent.rerunning {
                        job = await controller.rerun(previous, confirmed: true)
                    } else {
                        job = await controller.deploy(project: intent.project, env: intent.env, confirmed: true)
                    }
                    if let job { navigation.showJob(job) }
                }
            })
        }
        .onChange(of: controller.store.projectNames) { _, names in
            if navigation.selection == nil, let first = names.first { navigation.selection = .project(first) }
        }
        .onChange(of: selectedEnvironmentOptions, initial: true) { _, _ in
            navigation.reconcileEnvs(in: controller.store)
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch controller.connection {
        case .connected:
            switch navigation.selection {
            case .project(let name):
                ProjectDetailView(project: name, controller: controller, navigation: navigation)
            case .ports:
                PortsView(ports: controller.store.ports, runs: controller.store.runs,
                          conflicts: controller.store.conflicts.keys.sorted().flatMap { controller.store.conflicts[$0] ?? [] },
                          onShowService: { navigation.showLogs(for: $0) })
            case .jobs:
                JobsContainer(controller: controller, navigation: navigation)
            case .owners:
                OwnersContainer(controller: controller, navigation: navigation)
            case nil:
                DetailEmptyState("No Project Selected", systemImage: "sidebar.left",
                                 description: "Add a directory with rocket.yaml, or run `rocket up` in one.")
            }
        default:
            ConnectionStateView(state: controller.connection, home: controller.home.path,
                                onRetry: { Task { await controller.connect() } })
        }
    }
}
