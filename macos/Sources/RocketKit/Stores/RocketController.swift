import Foundation
import Observation
import os

/// Connection lifecycle shown in the UI.
public enum ConnectionState: Equatable, Sendable {
    case idle
    case connecting(String)
    case connected(HealthInfo)
    case failed(String)

    public var isConnected: Bool {
        if case .connected = self { return true }
        return false
    }
}

/// Owns the daemon connection: loads daemon.json, starts rocketd when
/// needed, keeps `store` in sync over HTTP + SSE and runs user actions.
@Observable
@MainActor
public final class RocketController {
    public let store: RocketStore
    public private(set) var connection: ConnectionState = .idle
    public private(set) var home: URL
    public private(set) var config: DaemonConfig?
    public private(set) var configWarnings: [String] = []
    public private(set) var streamLive = false
    public private(set) var eventsReceived = 0
    /// In-flight action keys, used to show progress and disable buttons.
    public private(set) var busy: Set<String> = []
    /// Last action error, surfaced as an alert.
    public var lastError: String?

    @ObservationIgnored private var client: RocketClient?
    @ObservationIgnored private var eventsTask: Task<Void, Never>?
    @ObservationIgnored private var jobLogTasks: [UUID: Task<Void, Never>] = [:]
    @ObservationIgnored private let log = Logger(subsystem: "com.xean.rocket", category: "controller")

    public init(store: RocketStore = RocketStore(), home: URL = DaemonConfigLoader.home(), client: RocketClient? = nil) {
        self.store = store
        self.home = home
        self.client = client
    }

    public var configURL: URL { DaemonConfigLoader.configURL(home: home) }

    // MARK: Connection

    /// Connects to rocketd, launching it through the CLI when unreachable.
    public func connect(launchIfNeeded: Bool = true) async {
        eventsTask?.cancel()
        cancelJobLogFollows()
        client = nil
        streamLive = false
        connection = .connecting("Connecting to rocketd…")
        if let health = await tryHealth() {
            await didConnect(health)
            return
        }
        guard launchIfNeeded else {
            connection = .failed("rocketd is not reachable at \(configURL.path).")
            return
        }
        guard let rocket = DaemonLauncher.locateRocket(config: config) else {
            connection = .failed(DaemonLauncher.LaunchError.notFound.localizedDescription)
            return
        }
        connection = .connecting("Starting rocketd…")
        log.info("starting rocketd with \(rocket.path, privacy: .public)")
        do {
            try await DaemonLauncher.startDaemon(rocket: rocket, home: home)
        } catch {
            connection = .failed(error.localizedDescription)
            return
        }
        for delay in [0.2, 0.4, 0.8, 1.0, 1.5, 2.0, 3.0] {
            if let health = await tryHealth() {
                await didConnect(health)
                return
            }
            try? await Task.sleep(for: .seconds(delay))
        }
        connection = .failed("rocketd started but did not answer. Check \(home.appendingPathComponent("rocketd.log").path).")
    }

    public func disconnect() {
        eventsTask?.cancel()
        eventsTask = nil
        cancelJobLogFollows()
        streamLive = false
        client = nil
        connection = .idle
    }

    /// Restarts rocketd through the CLI, then reconnects.
    public func restartDaemon() async {
        guard let rocket = DaemonLauncher.locateRocket(config: config) else {
            lastError = DaemonLauncher.LaunchError.notFound.localizedDescription
            return
        }
        disconnect()
        connection = .connecting("Restarting rocketd…")
        try? await DaemonLauncher.stopDaemon(rocket: rocket, home: home)
        await connect(launchIfNeeded: true)
    }

    private func tryHealth() async -> HealthInfo? {
        do {
            let loaded = try DaemonConfigLoader.load(from: configURL)
            config = loaded.config
            configWarnings = loaded.warnings
            for w in loaded.warnings { log.warning("\(w, privacy: .public)") }
            let client = RocketClient(config: loaded.config)
            let health = try await client.health()
            self.client = client
            return health
        } catch {
            log.info("health failed: \(error.localizedDescription, privacy: .public)")
            return nil
        }
    }

    private func didConnect(_ health: HealthInfo) async {
        connection = .connected(health)
        log.info("connected to rocketd \(health.version, privacy: .public) pid \(health.pid)")
        await refreshAll()
        startEvents()
    }

    private func startEvents() {
        guard let client else { return }
        eventsTask?.cancel()
        let stream = client.events()
        // URLSession holds an SSE response until the first body bytes arrive
        // (rocketd's first ping can be 15s away), so assume live until a drop.
        streamLive = true
        eventsTask = Task { [weak self] in
            var drops = 0
            for await update in stream.updates() {
                guard let self, !Task.isCancelled else { return }
                switch update {
                case .connected:
                    drops = 0
                    self.streamLive = true
                    // Slow consumers may miss events: re-sync on every (re)connect.
                    await self.refreshAll()
                case .event(let event):
                    self.eventsReceived += 1
                    self.store.apply(event)
                case .ping:
                    self.streamLive = true
                case .disconnected(let reason):
                    self.streamLive = false
                    drops += 1
                    self.log.info("event stream dropped: \(reason, privacy: .public)")
                    // rocketd may have restarted on a new port/token.
                    if drops >= 3, self.configChangedOnDisk() {
                        Task { await self.connect(launchIfNeeded: false) }
                        return
                    }
                }
            }
        }
    }

    private func configChangedOnDisk() -> Bool {
        guard let current = try? DaemonConfigLoader.load(from: configURL).config else { return false }
        return current.http != config?.http || current.token != config?.token
    }

    // MARK: Snapshots

    /// Reloads every snapshot (also called after each SSE reconnect).
    public func refreshAll() async {
        guard let client else { return }
        async let projects = client.projects()
        async let ps = client.ps(all: true)
        async let ports = client.ports()
        do {
            store.projects = try await projects.projects
            store.replaceRuns(try await ps.services, project: nil)
            store.ports = try await ports.ports.sorted { $0.port < $1.port }
        } catch {
            report(error)
            return
        }
        for project in store.projects {
            await refreshProject(project.name)
        }
        await refreshJobs()
    }

    /// Loads one project's declared services, default env and conflicts.
    public func refreshProject(_ project: String) async {
        guard let client else { return }
        do {
            store.applySummary(try await client.status(project: project), project: project)
        } catch RocketError.endpointUnavailable {
            // Older rocketd without /v1/status: ps also lists declared services.
            if let ps = try? await client.ps(project: project) { store.replaceRuns(ps.services, project: project) }
        } catch {
            report(error)
        }
    }

    public func refreshJobs() async {
        guard let client else { return }
        let revision = store.jobsRevision
        do {
            store.setJobs(try await client.jobs(project: nil).jobs, ifUnchangedSince: revision)
            store.jobsAvailable = true
        } catch RocketError.endpointUnavailable {
            store.jobsAvailable = false
        } catch {
            log.info("jobs refresh failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    public func loadLogs(_ key: ServiceKey) async {
        guard let client else { return }
        do {
            store.setLogs(try await client.logs(project: key.project, service: key.service, tail: 300).lines, for: key)
        } catch {
            log.info("logs failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    public func loadJobLogs(_ id: String) async {
        guard let client else { return }
        let source = store.beginJobLogFollow(id)
        if let lines = try? await client.jobLogs(id: id, tail: 300).lines {
            store.setFollowedJobLogs(lines, for: id, source: source)
        }
    }

    /// Both the selecting view and the connection lifecycle can cancel this source.
    public func followJobLogs(_ id: String) async {
        guard let client, !Task.isCancelled else { return }
        let source = store.beginJobLogFollow(id)
        let task = Task { [weak self] in
            guard let self else { return }
            await consumeJobLogs(id, source: source, client: client)
        }
        jobLogTasks[source] = task
        defer { jobLogTasks[source] = nil }
        await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            task.cancel()
        }
    }

    private func cancelJobLogFollows() {
        for task in jobLogTasks.values { task.cancel() }
        jobLogTasks.removeAll()
    }

    private func consumeJobLogs(_ id: String, source: UUID, client: RocketClient) async {
        do {
            try Task.checkCancellation()
            for try await event in client.jobLogStream(id: id).events() {
                try Task.checkCancellation()
                guard event.jobId == id || event.job?.id == id else { continue }
                if event.type == .jobLog, let line = event.line {
                    store.appendFollowedJobLog(line, for: id, source: source)
                } else if event.type == .jobState {
                    store.apply(event)
                }
            }
        } catch {
            guard !Task.isCancelled else { return }
            log.info("job log follow failed: \(error.localizedDescription, privacy: .public)")
            lastError = error.localizedDescription
        }
    }

    // MARK: Actions

    public func up(project: String, services: [String]? = nil, env: String? = nil) async {
        await perform("up:\(project)") { client in
            let result = try await client.up(UpRequest(project: project, services: services, env: env))
            if let failures = result.failureSummary { self.lastError = failures }
        }
        await refreshProject(project)
    }

    public func restart(project: String, services: [String]? = nil, env: String? = nil) async {
        await perform("restart:\(project)") { client in
            let result = try await client.restart(UpRequest(project: project, services: services, env: env))
            if let failures = result.failureSummary { self.lastError = failures }
        }
        await refreshProject(project)
    }

    public func down(project: String, services: [String]? = nil) async {
        await perform("down:\(project)") { client in
            let result = try await client.down(DownRequest(project: project, services: services))
            if let errors = result.errors, !errors.isEmpty { self.lastError = errors.joined(separator: "\n") }
        }
        await refreshProject(project)
    }

    public func stopAll(owner: String? = nil) async {
        await perform("down:*") { client in
            let result = try await client.down(DownRequest(project: nil, owner: owner, everywhere: true))
            if let errors = result.errors, !errors.isEmpty { self.lastError = errors.joined(separator: "\n") }
        }
        await refreshAll()
    }

    public func gc() async {
        await perform("gc") { client in _ = try await client.gc() }
        await refreshAll()
    }

    public func addProject(path: String) async {
        await perform("project:add") { client in _ = try await client.addProject(path: path) }
        await refreshAll()
    }

    public func removeProject(_ name: String) async {
        await perform("project:rm:\(name)") { client in try await client.removeProject(name: name) }
        await refreshAll()
    }

    /// Starts the same job again. Deploys need `confirmed` (the human said yes).
    @discardableResult
    public func rerun(_ job: Job, confirmed: Bool = false) async -> Job? {
        guard job.kind != .deploy || confirmed else { return nil }
        var req = JobRequest(rerunning: job)
        if job.kind == .deploy { req.yes = true }
        return await submitJob(req, key: "job:\(job.id)")
    }

    @discardableResult
    public func runPipeline(project: String, name: String, env: String? = nil) async -> Job? {
        await submitJob(JobRequest(project: project, kind: .pipeline, name: name, env: env),
                        key: "pipeline:\(project)")
    }

    @discardableResult
    public func deploy(project: String, env: String, confirmed: Bool = false) async -> Job? {
        guard confirmed else { return nil }
        return await submitJob(JobRequest(project: project, kind: .deploy, name: env, env: env, yes: true),
                               key: "deploy:\(project)")
    }

    private func submitJob(_ request: JobRequest, key: String) async -> Job? {
        guard let client else {
            lastError = "Not connected to rocketd."
            return nil
        }
        guard !busy.contains(key) else { return nil }
        busy.insert(key)
        defer { busy.remove(key) }
        do {
            let job = try await client.startJob(request)
            return store.upsertJob(job)
        } catch {
            report(error)
            return nil
        }
    }

    public func cancel(_ job: Job) async {
        await perform("job:\(job.id)") { client in _ = try await client.cancelJob(id: job.id) }
        await refreshJobs()
    }

    public func isBusy(_ key: String) -> Bool { busy.contains(key) }

    private func perform(_ key: String, _ body: (RocketClient) async throws -> Void) async {
        guard let client else {
            lastError = "Not connected to rocketd."
            return
        }
        busy.insert(key)
        defer { busy.remove(key) }
        do {
            try await body(client)
        } catch {
            report(error)
        }
    }

    private func report(_ error: Error) {
        log.error("\(error.localizedDescription, privacy: .public)")
        if case RocketError.unauthorized = error {
            // Token rotated (daemon restarted): re-read daemon.json, per the app contract.
            Task { await connect(launchIfNeeded: true) }
            return
        }
        lastError = error.localizedDescription
    }
}
