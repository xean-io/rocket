import RocketKit
import SwiftUI

/// Container for job history: binds store, actions and the log inspector.
struct JobsContainer: View {
    @Bindable var controller: RocketController
    @Bindable var navigation: AppNavigation
    @State private var logReload = 0

    private struct LogSelection: Hashable {
        var id: String?
        var reload: Int
    }

    private var store: RocketStore { controller.store }
    private var selected: Job? { store.jobs.first { $0.id == navigation.selectedJob } }

    var body: some View {
        Group {
            if store.jobsAvailable {
                JobsView(jobs: store.jobs, selection: $navigation.selectedJob,
                         onRerun: { job in
                             if job.kind == .deploy {
                                 navigation.confirmDeployment(project: job.project, env: job.env ?? job.name, rerunning: job)
                             } else {
                                 Task {
                                     if let newJob = await controller.rerun(job) { navigation.showJob(newJob) }
                                 }
                             }
                         },
                         onCancel: { job in Task { await controller.cancel(job) } })
            } else {
                DetailEmptyState("Jobs Not Available", systemImage: "list.bullet.rectangle",
                                 description: "This rocketd does not expose the jobs API yet. Update rocket and restart the daemon.")
            }
        }
        .navigationTitle(VerificationRuntime.title(for: "Jobs"))
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Refresh", systemImage: "arrow.clockwise") {
                    Task {
                        await controller.refreshJobs()
                        logReload += 1
                    }
                }
            }
        }
        .inspector(isPresented: $navigation.showInspector) {
            JobInspector(job: selected, logs: selected.flatMap { store.jobLogs[$0.id] } ?? [])
                .inspectorColumnWidth(min: 300, ideal: 420, max: 680)
        }
        .task(id: LogSelection(id: navigation.selectedJob, reload: logReload)) {
            if let id = navigation.selectedJob { await controller.followJobLogs(id) }
        }
    }
}

/// Presentational job history table.
struct JobsView: View {
    let jobs: [Job]
    @Binding var selection: Job.ID?
    let onRerun: (Job) -> Void
    let onCancel: (Job) -> Void

    var body: some View {
        VStack(spacing: 0) {
            SectionHeader(title: "Jobs", detail: "\(jobs.filter { $0.status == .running }.count) running · \(jobs.count) recent")
            Hairline()
            if jobs.isEmpty {
                DetailEmptyState("No Jobs Yet", systemImage: "list.bullet.rectangle",
                                 description: "Pipelines, setup and deploys started with rocket run, setup or deploy appear here.")
            } else {
                GeometryReader { geometry in
                    jobTable(compact: geometry.size.width < 780)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .overlay(alignment: .bottom) {
            if let job = jobs.first(where: { $0.id == selection }) {
                GlassEffectContainer {
                    HStack(spacing: 10) {
                        if job.status == .running {
                            Button("Cancel", systemImage: "stop.fill") { onCancel(job) }
                                .buttonStyle(.glass)
                        } else {
                            Button("Run Again", systemImage: "arrow.clockwise") { onRerun(job) }
                                .buttonStyle(.glassProminent)
                        }
                    }
                }
                .padding(.bottom, 18)
            }
        }
    }

    private func jobTable(compact: Bool) -> some View {
        Table(jobs, selection: $selection) {
            TableColumn("Job") { job in
                HStack(spacing: 8) {
                    StatusGlyph(style: StatusStyle(job.status))
                    Text(job.name.isEmpty ? job.kind.rawValue : job.name)
                        .foregroundStyle(Color.xeanInk)
                        .lineLimit(1)
                    if !compact { MetaLabel(job.kind.rawValue) }
                }
                .accessibilityElement(children: .combine)
            }
            .width(min: 160, ideal: 220)
            TableColumn("Project") { job in Text(job.project).lineLimit(1) }
                .width(min: 110, ideal: 150)
            if !compact {
                TableColumn("Step") { job in
                    Text(job.steps.isEmpty ? "—" : "\(job.step ?? 0)/\(job.steps.count)").monospacedDigit()
                }
                .width(min: 45, ideal: 60)
            }
            TableColumn("Exit") { job in
                Text(job.exitCode.map(String.init) ?? "—")
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle((job.exitCode ?? 0) == 0 ? Color.secondary : Color.red)
            }
            .width(min: 45, ideal: 50)
            if !compact {
                TableColumn("Owner") { job in MetaLabel(Format.owner(job.owner)) }
                    .width(min: 90, ideal: 120)
                TableColumn("Started") { job in
                    if let started = job.startedAt, started.timeIntervalSince1970 > 0 {
                        Text(started, format: .relative(presentation: .named)).foregroundStyle(.secondary)
                    } else {
                        Text("—")
                    }
                }
                .width(min: 100, ideal: 130)
            }
            TableColumn("Duration") { job in Text(Format.duration(ms: job.durationMs)).monospacedDigit() }
                .width(min: 75, ideal: 90)
        }
        .contextMenu(forSelectionType: Job.ID.self) { ids in
            if let job = jobs.first(where: { ids.contains($0.id) }) {
                if job.status == .running {
                    Button("Cancel Job", systemImage: "stop.circle", role: .destructive) { onCancel(job) }
                } else {
                    Button("Run Again", systemImage: "arrow.clockwise") { onRerun(job) }
                }
            }
        }
        .scrollContentBackground(.hidden)
        .background(Color.xeanSurface.opacity(0.6))
        .xeanTableSelection(selection: selection, rowCount: jobs.count)
    }
}

/// Job facts and log.
struct JobInspector: View {
    let job: Job?
    let logs: [String]

    var body: some View {
        if let job {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 8) {
                        StatusGlyph(style: StatusStyle(job.status)).font(.title3)
                        Text(job.name.isEmpty ? job.kind.rawValue : job.name).font(.title2)
                    }
                    MetaLabel("\(job.kind.rawValue) · \(job.project) · \(job.id)")
                    if let env = job.env { MetaLabel("Environment: \(env)") }
                    MetaLabel("Owner: \(Format.owner(job.owner))")
                    if let started = job.startedAt, started.timeIntervalSince1970 > 0 {
                        Text("Started: \(started.formatted(date: .abbreviated, time: .standard))")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(Array(job.steps.enumerated()), id: \.offset) { i, step in
                        HStack(spacing: 6) {
                            Image(systemName: (job.step ?? 0) > i + 1 || (job.status == .succeeded) ? "checkmark" : "circle")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Text(step.describe).font(.system(.callout, design: .monospaced))
                        }
                    }
                    if let error = job.error, !error.isEmpty {
                        Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled)
                    }
                }
                .padding(16)
                Hairline()
                LogTailView(lines: logs)
            }
        } else {
            DetailEmptyState("No Job Selected", systemImage: "sidebar.trailing")
        }
    }
}
