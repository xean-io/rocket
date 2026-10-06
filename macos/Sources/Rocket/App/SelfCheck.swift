import Foundation
import RocketKit

/// `Rocket --self-check [--timeout N]`: headless connectivity probe used for
/// verification. Connects (starting rocketd if needed), lists services,
/// waits for one SSE event and prints a summary. Exit 0 on success.
enum SelfCheck {
    static func run() {
        let args = CommandLine.arguments
        let timeout = args.firstIndex(of: "--timeout").flatMap { args.indices.contains($0 + 1) ? Double(args[$0 + 1]) : nil } ?? 30
        Task { @MainActor in
            let code = await check(timeout: timeout)
            exit(code)
        }
        dispatchMain()
    }

    @MainActor
    private static func check(timeout: Double) async -> Int32 {
        let controller = RocketController()
        say("home \(controller.home.path)")
        await controller.connect(launchIfNeeded: true)
        guard case .connected(let health) = controller.connection else {
            say("FAIL connect: \(controller.connection)")
            return 1
        }
        say("connected api=\(health.api) version=\(health.version) pid=\(health.pid)")
        for w in controller.configWarnings { say("warning \(w)") }
        let store = controller.store
        say("projects \(store.projectNames.joined(separator: ","))")
        say("services \(store.runs.count) running=\(store.runningCount)")
        for run in store.runs.values.sorted(by: { $0.id < $1.id }) {
            say("  \(run.id) \(run.state) ports=\(Format.ports(run.ports)) owner=\(Format.owner(run.owner))")
        }
        say("ports \(store.ports.map { String($0.port) }.joined(separator: ","))")
        say("jobs available=\(store.jobsAvailable) count=\(store.jobs.count)")

        let deadline = Date().addingTimeInterval(timeout)
        while controller.eventsReceived == 0, Date() < deadline {
            try? await Task.sleep(for: .milliseconds(200))
        }
        guard controller.eventsReceived > 0 else {
            say("FAIL no SSE event within \(Int(timeout))s (stream live=\(controller.streamLive))")
            return 2
        }
        say("events \(controller.eventsReceived) stream live=\(controller.streamLive)")

        // Re-sync and exercise the read endpoints the UI uses.
        await controller.refreshAll()
        say("jobs available=\(store.jobsAvailable) count=\(store.jobs.count)")
        if let run = store.runs.values.sorted(by: { $0.id < $1.id }).first(where: { $0.state.isActive }) {
            await controller.loadLogs(run.id)
            say("logs \(run.id) lines=\(store.logs[run.id]?.count ?? 0)")
        }
        if let job = store.jobs.first {
            await controller.loadJobLogs(job.id)
            say("job \(job.id) \(job.kind) \(job.name) \(job.status) exit=\(job.exitCode.map(String.init) ?? "-") lines=\(store.jobLogs[job.id]?.count ?? 0)")
        }
        if CommandLine.arguments.contains("--exercise"), let run = store.runs.values.first(where: { $0.state.isActive }) {
            // Drive the same POST paths the UI buttons use.
            let before = controller.eventsReceived
            await controller.restart(project: run.project, services: [run.service])
            say("restart \(run.id) -> \(store.runs[run.id]?.state.rawValue ?? "?") events +\(controller.eventsReceived - before)")
            if let job = store.jobs.first(where: { $0.kind == .pipeline }) {
                await controller.rerun(job)
                say("rerun job \(job.name) -> jobs \(store.jobs.count)")
            }
        }
        if let error = controller.lastError {
            say("FAIL action error: \(error)")
            return 3
        }
        say("OK")
        return 0
    }

    private static func say(_ s: String) {
        FileHandle.standardOutput.write(Data(("rocket-self-check: " + s + "\n").utf8))
    }
}
