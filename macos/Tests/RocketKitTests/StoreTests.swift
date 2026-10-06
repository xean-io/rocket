import Foundation
import Testing
@testable import RocketKit

@MainActor
@Suite("Store reducers")
struct StoreTests {
    private func event(_ json: String) throws -> DaemonEvent {
        try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(json.utf8))
    }

    private func seeded() throws -> RocketStore {
        let store = RocketStore()
        store.replaceRuns(try Fixture.decode(StatusResult.self, "ps.json").services, project: nil)
        return store
    }

    @Test func serviceStateUpsertsFullRun() throws {
        let store = try seeded()
        try store.apply(event(#"{"type":"service.state","project":"nuvara","service":"api","state":"starting","run":{"project":"nuvara","service":"api","state":"starting","pid":9,"owner":"agent:x"}}"#))
        let key = ServiceKey(project: "nuvara", service: "api")
        #expect(store.runs[key]?.state == .starting && store.runs[key]?.pid == 9)
        #expect(store.runningCount == 2) // static (running) + api (starting)
        #expect(store.projectNames == ["nuvara", "rocket-fixture"])
    }

    @Test func serviceStateWithoutRunPatchesState() throws {
        let store = try seeded()
        try store.apply(event(#"{"type":"service.state","project":"rocket-fixture","service":"static","state":"stopped"}"#))
        let run = store.runs[ServiceKey(project: "rocket-fixture", service: "static")]
        #expect(run?.state == .stopped && run?.pid == 8635)
        #expect(store.runningCount == 0)
        try store.apply(event(#"{"type":"service.state","project":"new","service":"svc","state":"running"}"#))
        #expect(store.runs[ServiceKey(project: "new", service: "svc")]?.state == .running)
    }

    @Test func logLinesAppendAndAreCapped() throws {
        let store = RocketStore(logLimit: 3)
        for i in 1...5 {
            try store.apply(event(#"{"type":"log.line","project":"p","service":"s","line":"l\#(i)"}"#))
        }
        #expect(store.logs[ServiceKey(project: "p", service: "s")] == ["l3", "l4", "l5"])
        store.setLogs(["a", "b"], for: ServiceKey(project: "p", service: "s"))
        #expect(store.logs[ServiceKey(project: "p", service: "s")] == ["a", "b"])
    }

    @Test func portLeaseEvents() throws {
        let store = RocketStore()
        store.ports = try Fixture.decode(PortsResult.self, "ports.json").ports
        try store.apply(event(#"{"type":"port.leased","project":"p","service":"s","lease":{"port":4000,"project":"p","service":"s","port_name":"http","created_at":"2026-10-05T00:14:10Z"}}"#))
        #expect(store.ports.map(\.port) == [3100, 4000, 18431])
        try store.apply(event(#"{"type":"port.released","project":"nuvara","service":"web","lease":{"port":3100,"project":"nuvara","service":"web","port_name":"http","created_at":"2026-10-05T00:14:10Z"}}"#))
        #expect(store.ports.map(\.port) == [4000, 18431])
    }

    @Test func replaceRunsScopedToProject() throws {
        let store = try seeded()
        try store.apply(event(#"{"type":"service.state","project":"nuvara","service":"api","state":"running"}"#))
        store.replaceRuns([], project: "rocket-fixture")
        #expect(store.runs.count == 1)
        #expect(store.runs(in: "nuvara").map(\.service) == ["api"])
    }

    @Test func ownersGroupActiveRuns() throws {
        let store = try seeded()
        try store.apply(event(#"{"type":"service.state","project":"nuvara","service":"api","state":"running","run":{"project":"nuvara","service":"api","state":"running","owner":"agent:t1"}}"#))
        let groups = store.ownerGroups
        #expect(groups.map(\.owner) == ["agent:t1", "user"])
        #expect(groups[0].runs.count == 1) // the exited db run is not active
        #expect(groups[0].isAgent)
    }

    @Test func summaryReplacesProjectRunsAndKeepsConflicts() throws {
        let store = try seeded()
        store.applySummary(try Fixture.decode(Summary.self, "summary.json"), project: "nuvara")
        #expect(store.runs(in: "nuvara").map(\.service) == ["web"])
        #expect(store.defaultEnvs["nuvara"] == "dev")
        #expect(store.conflicts["nuvara"]?.count == 2)
        #expect(store.knownEnvs(for: "nuvara") == ["dev"])
        #expect(store.runs(in: "rocket-fixture").count == 3)
    }

    @Test func jobEvents() throws {
        let store = RocketStore()
        try store.apply(event(#"{"type":"job.state","job_id":"j1","status":"running","job":{"id":"j1","project":"p","name":"ci","kind":"pipeline","owner":"user","steps":[],"status":"running","started_at":"2026-10-05T00:14:10Z","duration_ms":0}}"#))
        try store.apply(event(#"{"type":"job.state","job_id":"j2","status":"running","job":{"id":"j2","project":"p","name":"ci","kind":"pipeline","owner":"user","steps":[],"status":"running","started_at":"2026-10-05T00:15:10Z","duration_ms":0}}"#))
        try store.apply(event(#"{"type":"job.state","job_id":"j1","status":"succeeded"}"#))
        #expect(store.jobs.map(\.id) == ["j2", "j1"]) // newest first
        #expect(store.jobs[1].status == .succeeded)
        try store.apply(event(#"{"type":"job.log","job_id":"j1","line":"ok"}"#))
        #expect(store.jobLogs["j1"] == ["ok"])
    }
}
