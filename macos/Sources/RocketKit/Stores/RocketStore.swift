import Foundation
import Observation

/// Active runs grouped by who started them.
public struct OwnerGroup: Hashable, Sendable, Identifiable {
    public var owner: String
    public var runs: [Run]
    public var id: String { owner }
    public var isAgent: Bool { owner.hasPrefix("agent:") }
}

/// Client-side mirror of daemon state. Pure reducers only: no IO, so it is
/// fully testable; `RocketController` feeds it from HTTP and SSE.
@Observable
@MainActor
public final class RocketStore {
    public private(set) var runs: [ServiceKey: Run] = [:]
    public private(set) var logs: [ServiceKey: [String]] = [:]
    public var ports: [PortInfo] = []
    public var projects: [ProjectRef] = []
    public private(set) var jobs: [Job] = []
    /// Advances on local/event updates so an older asynchronous snapshot cannot replace them.
    public private(set) var jobsRevision: UInt64 = 0
    public private(set) var jobLogs: [String: [String]] = [:]
    @ObservationIgnored private var jobLogSources: [String: UUID] = [:]
    /// False when the daemon has no jobs API yet.
    public var jobsAvailable = true
    /// Per-project data from `GET /v1/status`.
    public private(set) var defaultEnvs: [String: String] = [:]
    /// Missing entries identify older daemons without declared environments.
    public private(set) var declaredEnvs: [String: [String]] = [:]
    public private(set) var declaredPipelines: [String: [String]] = [:]
    public private(set) var deployEnvs: [String: [String]] = [:]
    public private(set) var conflicts: [String: [Conflict]] = [:]

    public let logLimit: Int

    public init(logLimit: Int = 2_000) {
        self.logLimit = logLimit
    }

    // MARK: Snapshots

    /// Replaces runs for one project (or all runs when `project` is nil),
    /// e.g. after `GET /v1/ps` on (re)connect.
    public func replaceRuns(_ newRuns: [Run], project: String?) {
        if let project {
            runs = runs.filter { $0.key.project != project }
        } else {
            runs = [:]
        }
        for run in newRuns { runs[run.id] = run }
    }

    /// Applies a project summary: declared services (never-started ones are
    /// `stopped`), declared/default environments and port conflicts.
    public func applySummary(_ summary: Summary, project: String) {
        let name = summary.project?.name ?? project
        replaceRuns(summary.services, project: name)
        defaultEnvs[name] = summary.project?.defaultEnv
        declaredEnvs[name] = summary.project?.envs
        declaredPipelines[name] = summary.project?.pipelines
        deployEnvs[name] = summary.project?.deployEnvs
        conflicts[name] = summary.conflicts
        for job in summary.jobs { upsertJob(job) }
    }

    public func setLogs(_ lines: [String], for key: ServiceKey) {
        logs[key] = Array(lines.suffix(logLimit))
    }

    public func setJobs(_ newJobs: [Job], ifUnchangedSince revision: UInt64? = nil) {
        guard revision == nil || revision == jobsRevision else { return }
        jobs = Self.sortedJobs(newJobs.map(retainingTerminalJob))
        jobsRevision &+= 1
    }

    @discardableResult
    public func upsertJob(_ job: Job) -> Job {
        let job = retainingTerminalJob(job)
        jobs.removeAll { $0.id == job.id }
        jobs.append(job)
        jobs = Self.sortedJobs(jobs)
        jobsRevision &+= 1
        return job
    }

    /// A terminal SSE event can arrive before the POST's older running response.
    private func retainingTerminalJob(_ incoming: Job) -> Job {
        if !incoming.status.isTerminal,
           let current = jobs.first(where: { $0.id == incoming.id }), current.status.isTerminal {
            return current
        }
        return incoming
    }

    public func setJobLogs(_ lines: [String], for id: String) {
        jobLogs[id] = Array(lines.suffix(logLimit))
    }

    /// The ordered file stream owns this buffer, including after completion.
    /// Delayed global events must never append a second copy of its tail.
    @discardableResult
    public func beginJobLogFollow(_ id: String) -> UUID {
        let source = UUID()
        jobLogSources[id] = source
        jobLogs[id] = []
        return source
    }

    public func appendFollowedJobLog(_ line: String, for id: String, source: UUID) {
        guard jobLogSources[id] == source else { return }
        append(line, to: &jobLogs[id, default: []])
    }

    public func setFollowedJobLogs(_ lines: [String], for id: String, source: UUID) {
        guard jobLogSources[id] == source else { return }
        setJobLogs(lines, for: id)
    }

    // MARK: Events

    public func apply(_ event: DaemonEvent) {
        switch event.type {
        case .serviceState:
            applyServiceState(event)
        case .logLine:
            guard let project = event.project, let service = event.service, let line = event.line else { return }
            append(line, to: &logs[ServiceKey(project: project, service: service), default: []])
        case .portLeased:
            guard let lease = event.lease else { return }
            var info = ports.first { $0.port == lease.port } ?? PortInfo(lease: lease)
            info.project = lease.project
            info.service = lease.service
            info.portName = lease.portName
            ports.removeAll { $0.port == lease.port }
            ports.append(info)
            ports.sort { $0.port < $1.port }
        case .portReleased:
            guard let lease = event.lease else { return }
            ports.removeAll { $0.port == lease.port }
        case .jobState:
            applyJobState(event)
        case .jobLog:
            guard let id = event.jobId, let line = event.line, jobLogSources[id] == nil else { return }
            append(line, to: &jobLogs[id, default: []])
        default:
            break
        }
    }

    private func applyServiceState(_ event: DaemonEvent) {
        if let run = event.run {
            runs[run.id] = run
            return
        }
        guard let project = event.project, let service = event.service, let state = event.state else { return }
        let key = ServiceKey(project: project, service: service)
        var run = runs[key] ?? Run(project: project, service: service, state: state)
        run.state = state
        runs[key] = run
    }

    private func applyJobState(_ event: DaemonEvent) {
        if let job = event.job {
            upsertJob(job)
        } else if let id = event.jobId, let status = event.status, let i = jobs.firstIndex(where: { $0.id == id }) {
            guard !jobs[i].status.isTerminal || status.isTerminal else { return }
            jobs[i].status = status
            jobsRevision &+= 1
        }
        jobs = Self.sortedJobs(jobs)
    }

    private func append(_ line: String, to buffer: inout [String]) {
        buffer.append(line)
        if buffer.count > logLimit { buffer.removeFirst(buffer.count - logLimit) }
    }

    private static func sortedJobs(_ jobs: [Job]) -> [Job] {
        jobs.sorted { ($0.startedAt ?? .distantPast, $0.id) > ($1.startedAt ?? .distantPast, $1.id) }
    }

    // MARK: Derived

    public var runningCount: Int { runs.values.filter { $0.state.isActive }.count }

    /// Registered projects plus any project with known runs, sorted.
    public var projectNames: [String] {
        Set(projects.map(\.name)).union(runs.keys.map(\.project)).sorted()
    }

    public func runs(in project: String) -> [Run] {
        runs.values.filter { $0.project == project }.sorted { $0.service < $1.service }
    }

    public func activeCount(in project: String) -> Int {
        runs.values.filter { $0.project == project && $0.state.isActive }.count
    }

    public var ownerGroups: [OwnerGroup] {
        let active = runs.values.filter { $0.state.isActive }
        return Dictionary(grouping: active) { $0.owner ?? "user" }
            .map { OwnerGroup(owner: $0.key, runs: $0.value.sorted { $0.id < $1.id }) }
            .sorted { $0.owner < $1.owner }
    }

    /// Declared environments are authoritative, including an empty list.
    /// Older daemons fall back to the default plus environments seen on runs.
    public func knownEnvs(for project: String) -> [String] {
        if let declared = declaredEnvs[project] { return declared.sorted() }
        var envs = Set(runs.values.filter { $0.project == project }.compactMap(\.env))
        if let d = defaultEnvs[project] { envs.insert(d) }
        return envs.sorted()
    }

    /// Validates an explicit selection; nil always means the project default.
    public func validatedEnv(_ selection: String?, for project: String) -> String? {
        guard let selection, knownEnvs(for: project).contains(selection) else { return nil }
        return selection
    }

    /// Reconciles selections across every project, even while another pane is open.
    public func validatedEnvironmentSelections(_ selections: [String: String]) -> [String: String] {
        selections.filter { validatedEnv($0.value, for: $0.key) != nil }
    }

    public func path(of project: String) -> String? {
        projects.first { $0.name == project }?.path
    }
}
