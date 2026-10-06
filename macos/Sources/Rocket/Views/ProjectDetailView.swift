import RocketKit
import SwiftUI

/// Container: binds one project's store state and actions to presentational views.
struct ProjectDetailView: View {
    let project: String
    @Bindable var controller: RocketController
    @Bindable var navigation: AppNavigation
    @State private var confirmStop = false

    private var store: RocketStore { controller.store }
    private var runs: [Run] { store.runs(in: project) }
    private var selectedRun: Run? {
        guard let key = navigation.selectedService, key.project == project else { return nil }
        return store.runs[key]
    }
    private var env: String? { navigation.env(for: project, in: store) }
    private var busy: Bool {
        ["up", "restart", "down"].contains { controller.isBusy("\($0):\(project)") }
    }

    var body: some View {
        VStack(spacing: 0) {
            ProjectHeader(name: project,
                          path: store.path(of: project),
                          runs: runs,
                          conflicts: store.conflicts[project] ?? [],
                          busy: busy)
            Hairline()
            if runs.isEmpty {
                DetailEmptyState("No Services", systemImage: "shippingbox",
                                 description: "rocket.yaml declares no services, or the project is not loaded yet.")
            } else {
                ServicesTable(runs: runs, selection: Binding(
                    get: { navigation.selectedService?.project == project ? navigation.selectedService : nil },
                    set: { navigation.selectedService = $0 }
                ), onShowLogs: { navigation.showLogs(for: $0) })
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .overlay(alignment: .bottom) {
            ServiceActionBar(
                selected: selectedRun,
                busy: busy,
                onUp: { act { await controller.up(project: project, services: targets, env: env) } },
                onRestart: { act { await controller.restart(project: project, services: targets, env: env) } },
                onStop: {
                    if targets == nil { confirmStop = true } else { act { await controller.down(project: project, services: targets) } }
                },
                onLogs: { navigation.showInspector.toggle() },
                onClearSelection: { navigation.selectedService = nil }
            )
            .padding(.bottom, 18)
        }
        .navigationTitle(VerificationRuntime.title(for: project))
        .toolbar { toolbar }
        .confirmationDialog("Stop every service in \(project)?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button("Stop All", role: .destructive) {
                act { await controller.down(project: project) }
            }
        } message: {
            Text("Compose projects are taken down too. Running jobs of this project are canceled.")
        }
        .inspector(isPresented: $navigation.showInspector) {
            ServiceInspector(run: selectedRun,
                             logs: selectedRun.flatMap { store.logs[$0.id] } ?? [],
                             onReload: { if let key = selectedRun?.id { Task { await controller.loadLogs(key) } } })
                .inspectorColumnWidth(min: 300, ideal: 380, max: 620)
        }
        .task(id: project) { await controller.refreshProject(project) }
        .task(id: navigation.selectedService) {
            if let key = navigation.selectedService, key.project == project { await controller.loadLogs(key) }
        }
    }

    /// Selected service, or nil for the whole project.
    private var targets: [String]? { selectedRun.map { [$0.service] } }

    private func act(_ work: @escaping @MainActor () async -> Void) {
        Task { await work() }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Menu {
                Picker("Environment", selection: Binding(
                    get: { env ?? "" },
                    set: { navigation.envs[project] = $0.isEmpty ? nil : $0 }
                )) {
                    Text(store.defaultEnvs[project].map { "Default (\($0))" } ?? "Default").tag("")
                    ForEach(store.knownEnvs(for: project), id: \.self) { Text($0).tag($0) }
                }
                .pickerStyle(.inline)
            } label: {
                Label(env ?? store.defaultEnvs[project] ?? "default", systemImage: "square.3.layers.3d")
            }
            .labelStyle(.titleAndIcon)
            .fixedSize()
            .help("Environment used by Up and Restart")
            .accessibilityLabel("Environment: \(env ?? store.defaultEnvs[project] ?? "default")")
        }
        ToolbarItem(placement: .primaryAction) {
            Button("Up", systemImage: "play.fill") {
                act { await controller.up(project: project, services: targets, env: env) }
            }
            .buttonStyle(.glassProminent)
            .help(targets == nil ? "Start every service" : "Start the selected service")
        }
        ToolbarSpacer(.fixed, placement: .primaryAction)
        ToolbarItemGroup(placement: .primaryAction) {
            Button("Restart", systemImage: "arrow.clockwise") {
                act { await controller.restart(project: project, services: targets, env: env) }
            }
            .help("Restart (⌘R)")
            Button("Down", systemImage: "stop.fill") {
                if targets == nil { confirmStop = true } else { act { await controller.down(project: project, services: targets) } }
            }
            .help("Stop (⌘.)")
        }
        ToolbarSpacer(.fixed, placement: .primaryAction)
        ToolbarItemGroup(placement: .primaryAction) {
            Menu("Run Pipeline", systemImage: "list.bullet.rectangle") {
                ForEach(store.declaredPipelines[project] ?? [], id: \.self) { name in
                    Button(name) {
                        act {
                            if let job = await controller.runPipeline(project: project, name: name, env: env) {
                                navigation.showJob(job)
                            }
                        }
                    }
                }
            }
            .labelStyle(.titleAndIcon)
            .disabled(!controller.connection.isConnected || !store.jobsAvailable ||
                      (store.declaredPipelines[project] ?? []).isEmpty || controller.isBusy("pipeline:\(project)"))
            .help(store.declaredPipelines[project] == nil ? "Pipeline list unavailable. Update Rocket to see configured pipelines."
                  : "Run a configured pipeline in the selected environment")
            Menu("Deploy…", systemImage: "paperplane") {
                ForEach(store.deployEnvs[project] ?? [], id: \.self) { target in
                    Button(target) { navigation.confirmDeployment(project: project, env: target) }
                }
            }
            .labelStyle(.titleAndIcon)
            .disabled(!controller.connection.isConnected || !store.jobsAvailable ||
                      (store.deployEnvs[project] ?? []).isEmpty || controller.isBusy("deploy:\(project)"))
            .help("Deploy to a configured environment after confirmation")
        }
        ToolbarSpacer(.fixed, placement: .primaryAction)
        ToolbarItem(placement: .primaryAction) {
            Button("Logs", systemImage: "sidebar.trailing") { navigation.showInspector.toggle() }
                .help("Show service details and logs (⌘L)")
        }
    }
}

/// Project title, path and status facts on the content layer.
struct ProjectHeader: View {
    let name: String
    let path: String?
    let runs: [Run]
    let conflicts: [Conflict]
    let busy: Bool

    private var running: Int { runs.filter { $0.state == .running }.count }
    private var failing: Int { runs.filter { [.failed, .dead, .exited].contains($0.state) }.count }
    private var transitioning: Int { runs.filter { $0.state == .starting || $0.state == .stopping }.count }

    var body: some View {
        HStack(alignment: .bottom, spacing: 16) {
            VStack(alignment: .leading, spacing: 6) {
                Text(name)
                    .font(.system(.largeTitle, weight: .regular))
                    .foregroundStyle(Color.xeanInk)
                    .lineLimit(1)
                if let path { MetaLabel(path) }
            }
            Spacer(minLength: 12)
            // Full labels when there is room; counts only when the sidebar
            // and inspector squeeze the header, never wrapped text.
            ViewThatFits(in: .horizontal) {
                pills(compact: false)
                pills(compact: true)
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 20)
        .padding(.bottom, 14)
    }

    private func pills(compact: Bool) -> some View {
        HStack(spacing: 8) {
            if busy {
                StatusPill(text: "Working", symbol: "circle.dotted", color: .xeanViolet, pulses: true, compact: compact)
            }
            StatusPill(text: "\(running) running", symbol: "circle.fill", color: running > 0 ? .green : .secondary,
                       compact: compact, value: running)
            if transitioning > 0 {
                StatusPill(text: "\(transitioning) changing", symbol: "circle.dotted", color: .orange, pulses: true,
                           compact: compact, value: transitioning)
            }
            if failing > 0 {
                StatusPill(text: "\(failing) down", symbol: "exclamationmark.triangle.fill", color: .red,
                           compact: compact, value: failing)
            }
            if !conflicts.isEmpty {
                StatusPill(text: "\(conflicts.count) port \(conflicts.count == 1 ? "issue" : "issues")",
                           symbol: "arrow.left.arrow.right", color: .orange, compact: compact, value: conflicts.count)
                    .help(conflicts.compactMap(\.detail).joined(separator: "\n"))
            }
        }
    }
}

/// A plain capsule carrying one status fact; glass is reserved for controls.
struct StatusPill: View {
    let text: String
    let symbol: String
    let color: Color
    var pulses = false
    /// Compact pills show only the count (or just the icon) and keep the full text as help.
    var compact = false
    var value: Int?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Label {
            if !compact {
                Text(text).monospacedDigit()
            } else if let value {
                Text("\(value)").monospacedDigit()
            }
        } icon: {
            Image(systemName: symbol)
                .foregroundStyle(color)
                .symbolEffect(.pulse, options: .repeating, isActive: pulses && !reduceMotion)
        }
        .font(.callout)
        .lineLimit(1)
        .fixedSize()
        .padding(.horizontal, compact ? 10 : 12)
        .padding(.vertical, 6)
        .background(Color.xeanSurface.opacity(0.8), in: Capsule())
        .overlay(Capsule().stroke(Color.xeanHairline, lineWidth: 0.5))
        .help(compact ? text : "")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
    }
}
