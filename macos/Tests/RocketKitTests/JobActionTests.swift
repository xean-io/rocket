import Foundation
import Synchronization
import Testing
@testable import RocketKit

@MainActor
@Suite("Job submission and deployment confirmation")
struct JobActionTests {
    private let runningJSON = #"{"id":"j-new","project":"p","kind":"pipeline","name":"check","env":"smoke","status":"running"}"#

    private func job(_ json: String) throws -> Job {
        try RocketJSON.decoder.decode(Job.self, from: Data(json.utf8))
    }

    private func controller(port: Int, status: Int = 200, response: String? = nil) -> RocketController {
        let body = Data((response ?? runningJSON).utf8)
        MockURLProtocol.register(port: port) { request in
            if request.httpMethod == "POST", request.url?.path == "/v1/jobs" { return (status, body) }
            return (200, Data(#"{"jobs":[]}"#.utf8))
        }
        return RocketController(home: URL(fileURLWithPath: "/tmp/rkj-\(port)"),
                                client: RocketClient(config: testConfig(port: port), session: MockURLProtocol.session()))
    }

    private func body(port: Int) throws -> [String: Any] {
        let request = try #require(MockURLProtocol.requests(port: port).first { $0.httpMethod == "POST" })
        let data = try #require(request.httpBody)
        return try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    @Test func pipelineSubmitsSelectedEnvironmentAndRetainsReturnedJobImmediately() async throws {
        let port = 42101
        let controller = controller(port: port)
        let returned = await controller.runPipeline(project: "p", name: "check", env: "smoke")
        #expect(returned?.id == "j-new")
        #expect(controller.store.jobs.first?.id == "j-new")
        let request = try body(port: port)
        #expect(request["project"] as? String == "p" && request["kind"] as? String == "pipeline")
        #expect(request["name"] as? String == "check" && request["env"] as? String == "smoke")
        #expect(request["owner"] as? String == "user" && request["yes"] == nil)
        #expect(MockURLProtocol.requests(port: port).count == 1)
    }

    @Test func failedSubmissionReturnsNoJobForNavigation() async {
        let port = 42102
        let controller = controller(port: port, status: 400, response: #"{"error":"unknown pipeline","code":"invalid"}"#)
        #expect(await controller.runPipeline(project: "p", name: "missing") == nil)
        #expect(controller.store.jobs.isEmpty)
        #expect(controller.lastError == "unknown pipeline")
        #expect(MockURLProtocol.requests(port: port).count == 1)
    }

    @Test func canceledNewDeploymentSubmitsNothingAndConfirmationSendsYes() async throws {
        let port = 42103
        let controller = controller(port: port, response: #"{"id":"j-deploy","project":"p","kind":"deploy","name":"stage","env":"stage","status":"running"}"#)
        #expect(await controller.deploy(project: "p", env: "stage") == nil)
        #expect(await controller.deploy(project: "p", env: "stage", confirmed: false) == nil)
        #expect(MockURLProtocol.requests(port: port).isEmpty)
        #expect(await controller.deploy(project: "p", env: "stage", confirmed: true)?.id == "j-deploy")
        let request = try body(port: port)
        #expect(request["kind"] as? String == "deploy" && request["name"] as? String == "stage")
        #expect(request["env"] as? String == "stage" && request["yes"] as? Bool == true)
        #expect(request["owner"] as? String == "user" && request["profiles"] == nil)
        #expect(controller.store.jobs.first?.id == "j-deploy")
        #expect(MockURLProtocol.requests(port: port).count == 1)
    }

    @Test func deploymentRerunCannotSubmitThroughAnUnconfirmedCallPath() async throws {
        let port = 42104
        let controller = controller(port: port, response: #"{"id":"j-deploy","project":"p","kind":"deploy","name":"stage","env":"stage","status":"running"}"#)
        let old = try job(#"{"id":"j-old","project":"p","kind":"deploy","name":"legacy-name","env":"stage","profiles":["legacy"],"args":["--check"],"status":"succeeded"}"#)
        #expect(await controller.rerun(old) == nil)
        #expect(MockURLProtocol.requests(port: port).isEmpty)
        #expect(await controller.rerun(old, confirmed: true)?.id == "j-deploy")
        let request = try body(port: port)
        #expect(request["name"] as? String == "stage" && request["env"] as? String == "stage")
        #expect(request["profiles"] == nil && request["yes"] as? Bool == true)
        #expect(request["args"] as? [String] == ["--check"])
        #expect(MockURLProtocol.requests(port: port).count == 1)
    }

    @Test(arguments: ["pipeline", "setup"])
    func nondeployRerunPreservesEnvironmentProfilesArgsAndChangesOwnerToUser(kind: String) async throws {
        let port = kind == "pipeline" ? 42105 : 42108
        let controller = controller(port: port, response: #"{"id":"j-new","project":"p","kind":"\#(kind)","name":"check","env":"smoke","status":"running"}"#)
        let old = try job(#"{"id":"j-old","project":"p","kind":"\#(kind)","name":"check","env":"smoke","profiles":["extra","trends"],"args":["--fast"],"owner":"agent:old","status":"failed"}"#)
        #expect(await controller.rerun(old)?.id == "j-new")
        let request = try body(port: port)
        #expect(request["env"] as? String == "smoke" && request["profiles"] as? [String] == ["extra", "trends"])
        #expect(request["args"] as? [String] == ["--fast"] && request["owner"] as? String == "user")
        #expect(request["kind"] as? String == kind)
        #expect(request["yes"] == nil)
        #expect(controller.store.jobs.first?.id == "j-new")
    }

    @Test func terminalEventWinsOverAnOlderRunningPostResponse() async throws {
        let port = 42106
        let controller = controller(port: port)
        let terminal = try job(#"{"id":"j-new","project":"p","kind":"pipeline","name":"check","env":"smoke","status":"succeeded","exit_code":0,"finished_at":"2026-10-05T00:15:10Z"}"#)
        let event = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.state","job":{"id":"j-new","project":"p","kind":"pipeline","name":"check","env":"smoke","status":"succeeded","exit_code":0,"finished_at":"2026-10-05T00:15:10Z"}}"#.utf8))
        controller.store.apply(event)
        let effective = await controller.runPipeline(project: "p", name: "check", env: "smoke")
        #expect(effective == terminal)
        #expect(controller.store.jobs == [terminal])
        #expect(controller.store.upsertJob(try job(runningJSON)) == terminal)
        controller.store.setJobs([try job(runningJSON)])
        #expect(controller.store.jobs == [terminal])
    }

    @Test func pipelineAndDeployMetadataKeepAbsenceDistinctFromEmpty() throws {
        for json in [
            #"{"name":"p","pipelines":["check","slow-check"],"deploy_envs":["stage"]}"#,
            #"{"name":"p","pipelines":[],"deploy_envs":[]}"#,
            #"{"name":"p"}"#,
        ] {
            let info = try RocketJSON.decoder.decode(ProjectInfo.self, from: Data(json.utf8))
            let encoded = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(info)) as? [String: Any]
            let original = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
            #expect(encoded?["pipelines"] as? [String] == original?["pipelines"] as? [String])
            #expect(encoded?["deploy_envs"] as? [String] == original?["deploy_envs"] as? [String])
        }
    }

    @Test func jobProfilesAndExpiryDecodeBackwardCompatibly() throws {
        let modern = try job(#"{"id":"j","env":"smoke","profiles":["trends"],"expires_at":"2026-10-05T00:15:10Z"}"#)
        let encoded = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(modern)) as? [String: Any]
        #expect(encoded?["profiles"] as? [String] == ["trends"])
        #expect(encoded?["expires_at"] != nil)
        let old = try job(#"{"id":"old"}"#)
        let oldEncoded = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(old)) as? [String: Any]
        #expect(old.env == nil && oldEncoded?["profiles"] == nil && oldEncoded?["expires_at"] == nil)
    }

    @Test func declaredActionListsRefreshAndOlderMetadataClearsThem() throws {
        let store = RocketStore()
        let modern = try RocketJSON.decoder.decode(Summary.self, from: Data(#"{"project":{"name":"p","pipelines":["check","slow-check"],"deploy_envs":["stage"]}}"#.utf8))
        store.applySummary(modern, project: "p")
        #expect(store.declaredPipelines["p"] == ["check", "slow-check"])
        #expect(store.deployEnvs["p"] == ["stage"])
        let empty = try RocketJSON.decoder.decode(Summary.self, from: Data(#"{"project":{"name":"p","pipelines":[],"deploy_envs":[]}}"#.utf8))
        store.applySummary(empty, project: "p")
        #expect(store.declaredPipelines["p"] == [] && store.deployEnvs["p"] == [])
        let older = try RocketJSON.decoder.decode(Summary.self, from: Data(#"{"project":{"name":"p"}}"#.utf8))
        store.applySummary(older, project: "p")
        #expect(store.declaredPipelines["p"] == nil && store.deployEnvs["p"] == nil)
    }

    @Test func jobRequestEncodesSelectionAndTTLOptionally() throws {
        let selected = JobRequest(project: "p", kind: .pipeline, name: "check", env: "smoke", profiles: ["trends"], ttl: "30m")
        let encoded = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(selected)) as? [String: Any]
        #expect(encoded?["env"] as? String == "smoke" && encoded?["profiles"] as? [String] == ["trends"])
        #expect(encoded?["ttl"] as? String == "30m")
        let defaults = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(JobRequest(project: "p", kind: .pipeline, name: "check"))) as? [String: Any]
        #expect(defaults?["env"] == nil && defaults?["profiles"] == nil && defaults?["ttl"] == nil)
        let legacyDeploy = try job(#"{"id":"old","project":"p","kind":"deploy","name":"stage","status":"succeeded"}"#)
        let rerun = JobRequest(rerunning: legacyDeploy)
        #expect(rerun.name == "stage" && rerun.env == "stage" && rerun.profiles == nil && rerun.yes == nil)
    }

    @Test func lateRunningEventsAndSummaryDoNotUndoTerminalState() throws {
        let store = RocketStore()
        let terminal = try job(#"{"id":"j-new","project":"p","status":"succeeded","exit_code":0}"#)
        store.upsertJob(terminal)
        let stale = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.state","job_id":"j-new","status":"running","job":{"id":"j-new","project":"p","status":"running"}}"#.utf8))
        store.apply(stale)
        #expect(store.jobs == [terminal])
        store.upsertJob(terminal)
        let patch = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.state","job_id":"j-new","status":"running"}"#.utf8))
        store.apply(patch)
        #expect(store.jobs == [terminal])
        store.upsertJob(terminal)
        let summary = try RocketJSON.decoder.decode(Summary.self, from: Data(#"{"project":{"name":"p"},"jobs":[{"id":"j-new","project":"p","status":"running"}]}"#.utf8))
        store.applySummary(summary, project: "p")
        #expect(store.jobs == [terminal])
    }

    @Test(arguments: ["submitted", "event"])
    func staleEmptyHistorySnapshotCannotRemoveANewerJob(source: String) async throws {
        let port = source == "submitted" ? 42109 : 42110
        let controller = controller(port: port)
        let snapshotStarted = Mutex(false)
        let releaseSnapshot = DispatchSemaphore(value: 0)
        MockURLProtocol.register(port: port) { _ in
            snapshotStarted.withLock { $0 = true }
            _ = releaseSnapshot.wait(timeout: .now() + 5)
            return (200, Data(#"{"jobs":[]}"#.utf8))
        }
        defer { releaseSnapshot.signal() }
        let refresh = Task { await controller.refreshJobs() }
        let deadline = Date().addingTimeInterval(2)
        while !snapshotStarted.withLock({ $0 }), Date() < deadline { await Task.yield() }
        try #require(snapshotStarted.withLock { $0 })
        let newer = try job(runningJSON)
        if source == "submitted" {
            controller.store.upsertJob(newer)
        } else {
            let event = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.state","job_id":"j-new","job":\#(runningJSON)}"#.utf8))
            controller.store.apply(event)
        }
        releaseSnapshot.signal()
        await refresh.value
        #expect(controller.store.jobs == [newer])
    }

    @Test func unchangedHistorySnapshotStillReplacesOldRows() async throws {
        let controller = controller(port: 42111)
        controller.store.upsertJob(try job(runningJSON))
        await controller.refreshJobs()
        #expect(controller.store.jobs.isEmpty)
        #expect(controller.store.jobsAvailable)
    }
}
