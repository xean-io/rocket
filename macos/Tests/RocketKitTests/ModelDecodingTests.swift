import Foundation
import Testing
@testable import RocketKit

@Suite("Go JSON decoding")
struct ModelDecodingTests {
    @Test func datesAcceptGoRFC3339NanoAndOffsets() throws {
        let plain = try #require(RocketJSON.parseDate("2026-10-05T00:14:10Z"))
        let nano = try #require(RocketJSON.parseDate("2026-10-05T00:14:10.938611Z"))
        let nine = try #require(RocketJSON.parseDate("2026-10-05T00:14:10.123456789Z"))
        let offset = try #require(RocketJSON.parseDate("2026-10-05T02:14:10+02:00"))
        #expect(abs(nano.timeIntervalSince(plain) - 0.938611) < 0.000_01)
        #expect(abs(nine.timeIntervalSince(plain) - 0.123456789) < 0.000_01)
        #expect(offset == plain)
        #expect(RocketJSON.parseDate("yesterday") == nil)
    }

    @Test func healthInfo() throws {
        let h = try Fixture.decode(HealthInfo.self, "health.json")
        #expect(h.ok && h.api == "v1" && h.pid == 4242)
        #expect(h.socket == "/Users/me/.rocket/rocketd.sock")
    }

    @Test func statusResultWithOmittedFields() throws {
        let ps = try Fixture.decode(StatusResult.self, "ps.json")
        #expect(ps.services.count == 3)
        let s = ps.services[0]
        #expect(s.id == ServiceKey(project: "rocket-fixture", service: "static"))
        #expect(s.state == .running && s.health == .healthy && s.kind == .run)
        #expect(s.ports == ["http": 18431])
        #expect(s.startedAt != nil && s.expiresAt == nil && s.exitCode == nil)
        let db = ps.services[2]
        #expect(db.state == .exited && db.exitCode == 2 && db.owner == "agent:t1")
        #expect(db.composeProject == "rocket-rocket-fixture-dev")
        #expect(ps.services[1].ports == nil && ps.services[1].owner == nil)
    }

    @Test func unknownEnumValuesDoNotBreakDecoding() throws {
        let json = #"{"project":"p","service":"s","state":"hibernating","health":"meh","kind":"lambda"}"#
        let run = try RocketJSON.decoder.decode(Run.self, from: Data(json.utf8))
        #expect(run.state.rawValue == "hibernating")
        #expect(!run.state.isActive)
        #expect(run.kind?.rawValue == "lambda")
    }

    @Test func upResultWithRemaps() throws {
        let up = try Fixture.decode(UpResult.self, "up_result.json")
        #expect(up.project == "nuvara" && up.env == "dev" && up.services.count == 3)
        let web = up.services[1]
        #expect(web.action == "started" && web.owner == "agent:claude-123")
        #expect(web.remaps?.first?.from == 3000 && web.remaps?.first?.to == 3100)
        #expect(web.remaps?.first?.holder?.command == "node")
        #expect(up.services[2].error?.contains("busy") == true)
    }

    @Test func otherShapes() throws {
        let down = try Fixture.decode(DownResult.self, "down_result.json")
        #expect(down.stopped.count == 1 && down.composeDown == ["rocket-nuvara-dev"])
        let logs = try Fixture.decode(LogsResult.self, "logs.json")
        #expect(logs.lines.count == 2)
        let ports = try Fixture.decode(PortsResult.self, "ports.json")
        #expect(ports.ports[0].portName == "http" && ports.ports[0].pid == 8635)
        #expect(ports.ports[1].owner == nil && ports.ports[1].state == nil)
        let gc = try Fixture.decode(GCResult.self, "gc.json")
        #expect(gc.actions.map(\.action) == ["released_lease", "pruned"])
        let projects = try Fixture.decode(ProjectsResult.self, "projects.json")
        #expect(projects.projects.first?.name == "nuvara")
        let err = try Fixture.decode(APIErrorBody.self, "error.json")
        #expect(err.code == "invalid")
        let job = try Fixture.decode(Job.self, "job.json")
        #expect(job.status == .failed && job.exitCode == 1 && job.steps.count == 2)
        #expect(job.steps[0].describe == "run step-lint" && job.steps[1].describe == "task e2e")
        #expect(job.durationMs == 60000)
    }

    @Test func summaryAndJobLogs() throws {
        let sum = try Fixture.decode(Summary.self, "summary.json")
        #expect(sum.project?.defaultEnv == "dev" && sum.project?.root == "/Users/me/code/nuvara")
        #expect(sum.services.count == 1 && sum.jobs.isEmpty)
        #expect(sum.conflicts.map(\.kind) == ["port_remapped", "port_busy"])
        #expect(sum.conflicts[0].default == 3000 && sum.conflicts[0].remappable == true)
        #expect(sum.conflicts[1].holder?.pid == 77)
        let logs = try Fixture.decode(JobLogsResult.self, "job_logs.json")
        #expect(logs.job == "j3f9a0c12be" && logs.lines.count == 2)
    }

    @Test func jobRequestMatchesContract() throws {
        let req = JobRequest(project: "nuvara", kind: .deploy, name: "prod", yes: true)
        let obj = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(req)) as? [String: Any]
        #expect(obj?["kind"] as? String == "deploy" && obj?["yes"] as? Bool == true)
        #expect(obj?["owner"] as? String == "user")
        #expect(obj?["allow_agent_deploy"] == nil)
    }

    @Test func upRequestEncodesSnakeCaseAndOmitsNil() throws {
        let req = UpRequest(project: "/code/x", services: ["api"])
        let obj = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(req)) as? [String: Any]
        #expect(obj?["project"] as? String == "/code/x")
        #expect(obj?["owner"] as? String == "user")
        #expect(obj?["env"] == nil && obj?["ttl"] == nil)
        let down = DownRequest(project: nil, services: nil, owner: "agent:x", everywhere: true)
        let dobj = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(down)) as? [String: Any]
        #expect(dobj?["everywhere"] as? Bool == true && dobj?["project"] == nil)
    }
}
