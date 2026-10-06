import Foundation
import Synchronization
import Testing
@testable import RocketKit

@Suite("Ordered job log follow")
struct JobLogFollowTests {
    private static let tail = "event: job.log\ndata: {\"type\":\"job.log\",\"job_id\":\"j\",\"line\":\"start\"}\n\n" +
        "event: job.log\ndata: {\"type\":\"job.log\",\"job_id\":\"j\",\"line\":\"repeat\"}\n\n" +
        "event: job.log\ndata: {\"type\":\"job.log\",\"job_id\":\"j\",\"line\":\"repeat\"}\n\n"
    private static let terminal = "event: job.state\ndata: {\"type\":\"job.state\",\"job_id\":\"j\",\"status\":\"succeeded\",\"job\":{\"id\":\"j\",\"status\":\"succeeded\"}}\n\n"

    @MainActor
    @Test func queuedGlobalEventsCannotReplayAnOrderedTailAndRepeatsRemain() throws {
        let store = RocketStore()
        store.setJobLogs(["old global buffer"], for: "j")
        let first = store.beginJobLogFollow("j")
        for line in ["start", "step", "repeat", "repeat"] { store.appendFollowedJobLog(line, for: "j", source: first) }
        for line in ["start", "step", "repeat", "repeat"] {
            let event = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.log","job_id":"j","line":"\#(line)"}"#.utf8))
            store.apply(event)
        }
        store.appendFollowedJobLog("trailing partial", for: "j", source: first)
        #expect(store.jobLogs["j"] == ["start", "step", "repeat", "repeat", "trailing partial"])
        // A new ordered subscription replaces its tail rather than concatenating it.
        let second = store.beginJobLogFollow("j")
        store.appendFollowedJobLog("repeat", for: "j", source: second)
        store.appendFollowedJobLog("stale stream", for: "j", source: first)
        store.setFollowedJobLogs(["stale snapshot"], for: "j", source: first)
        #expect(store.jobLogs["j"] == ["repeat"])
        let other = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.log","job_id":"other","line":"still global"}"#.utf8))
        store.apply(other)
        #expect(store.jobLogs["other"] == ["still global"])
    }

    @Test func finiteStreamKeepsRepeatedLinesAndDrainsBeforeOneTerminalWithoutReconnect() async throws {
        let calls = Mutex(0)
        let stream = JobLogStream(connect: {
            calls.withLock { $0 += 1 }
            let (chunks, producer) = AsyncThrowingStream<[UInt8], Error>.makeStream()
            for text in [String(Self.tail.prefix(13)), String(Self.tail.dropFirst(13)), ": ping\n\n",
                         "event: job.log\ndata: {\"type\":\"job.log\",\"job_id\":\"j\",\"line\":\"partial\"}\n\n",
                         Self.terminal, Self.tail] { producer.yield(Array(text.utf8)) }
            producer.finish()
            return JobLogStream.Connection(chunks: chunks, cancel: { producer.finish() })
        })
        var events: [DaemonEvent] = []
        for try await event in stream.events() { events.append(event) }
        #expect(events.compactMap(\.line) == ["start", "repeat", "repeat", "partial"])
        #expect(events.map(\.type) == [.jobLog, .jobLog, .jobLog, .jobLog, .jobState])
        #expect(events.last?.status == .succeeded && calls.withLock { $0 } == 1)
    }

    @Test func prematureEOFIsAnErrorAndDoesNotReplay() async throws {
        let calls = Mutex(0)
        let stream = JobLogStream(connect: {
            calls.withLock { $0 += 1 }
            let (chunks, producer) = AsyncThrowingStream<[UInt8], Error>.makeStream()
            producer.yield(Array(Self.tail.utf8))
            producer.finish()
            return JobLogStream.Connection(chunks: chunks, cancel: { producer.finish() })
        })
        var lines: [String] = []
        var failed = false
        do {
            for try await event in stream.events() { if let line = event.line { lines.append(line) } }
        } catch { failed = true }
        #expect(failed && lines == ["start", "repeat", "repeat"])
        #expect(calls.withLock { $0 } == 1)
    }

    @MainActor
    @Test func controllerUsesOneAuthenticatedOrderedSourceBeforeQueuedGlobalEvents() async throws {
        let store = RocketStore()
        let requests = Mutex<[URLRequest]>([])
        let client = RocketClient(config: testConfig(port: 42201, token: "job-fixture"), streamOpener: { request in
            requests.withLock { $0.append(request) }
            // These global events were already queued when the ordered tail opened.
            try await MainActor.run {
                for line in ["start", "repeat", "repeat"] {
                    let event = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"job.log","job_id":"j","line":"\#(line)"}"#.utf8))
                    store.apply(event)
                }
            }
            let (chunks, producer) = AsyncThrowingStream<[UInt8], Error>.makeStream()
            producer.yield(Array((Self.tail + Self.terminal).utf8))
            producer.finish()
            return JobLogStream.Connection(chunks: chunks, cancel: { producer.finish() })
        })
        let controller = RocketController(store: store, home: URL(fileURLWithPath: "/tmp/rkj-follow"), client: client)
        store.setJobLogs(["old snapshot"], for: "j")
        await controller.followJobLogs("j")
        #expect(store.jobLogs["j"] == ["start", "repeat", "repeat"])
        #expect(store.jobs.first?.status == .succeeded)
        let request = try #require(requests.withLock { $0.first })
        #expect(request.url?.path == "/v1/jobs/j/logs")
        let query = URLComponents(url: try #require(request.url), resolvingAgainstBaseURL: false)?.queryItems
        #expect(query?.contains(URLQueryItem(name: "follow", value: "true")) == true)
        #expect(query?.contains(URLQueryItem(name: "tail", value: "300")) == true)
        #expect(request.value(forHTTPHeaderField: "Accept") == "text/event-stream")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer job-fixture")
        #expect(requests.withLock { $0.count } == 1)
    }

    @Test(arguments: [false, true])
    func stoppingFollowClosesItsBodyStream(terminal: Bool) async throws {
        let opened = Mutex(false)
        let closed = Mutex(false)
        let (body, producer) = AsyncThrowingStream<[UInt8], Error>.makeStream()
        producer.onTermination = { _ in closed.withLock { $0 = true } }
        defer { producer.finish() }
        let stream = JobLogStream(connect: {
            opened.withLock { $0 = true }
            return JobLogStream.Connection(chunks: body, cancel: { producer.finish() })
        })
        let consumer = Task {
            do { for try await _ in stream.events() {} } catch {}
        }
        let deadline = Date().addingTimeInterval(1)
        while !opened.withLock({ $0 }), Date() < deadline { await Task.yield() }
        #expect(opened.withLock { $0 })
        if terminal { producer.yield(Array(Self.terminal.utf8)) } else { consumer.cancel() }
        await consumer.value
        let closeDeadline = Date().addingTimeInterval(1)
        while !closed.withLock({ $0 }), Date() < closeDeadline { await Task.yield() }
        #expect(closed.withLock { $0 })
    }

    @MainActor
    @Test(arguments: [false, true])
    func controllerRetainsPartialLogsAndCancellationDoesNotShowAnError(cancel: Bool) async throws {
        let calls = Mutex(0)
        let closed = Mutex(false)
        let (chunks, producer) = AsyncThrowingStream<[UInt8], Error>.makeStream()
        producer.onTermination = { _ in closed.withLock { $0 = true } }
        defer { producer.finish() }
        let client = RocketClient(config: testConfig(port: cancel ? 42202 : 42203), streamOpener: { _ in
            calls.withLock { $0 += 1 }
            producer.yield(Array(Self.tail.utf8))
            if !cancel { producer.finish() }
            return JobLogStream.Connection(chunks: chunks, cancel: { producer.finish() })
        })
        let controller = RocketController(home: URL(fileURLWithPath: "/tmp/rkj-partial"), client: client)
        let follow = Task { await controller.followJobLogs("j") }
        let deadline = Date().addingTimeInterval(1)
        while controller.store.jobLogs["j"]?.count != 3, Date() < deadline { await Task.yield() }
        #expect(controller.store.jobLogs["j"] == ["start", "repeat", "repeat"])
        if cancel { follow.cancel() }
        await follow.value
        #expect(cancel ? controller.lastError == nil : controller.lastError != nil)
        #expect(calls.withLock { $0 } == 1)
        let closeDeadline = Date().addingTimeInterval(1)
        while !closed.withLock({ $0 }), Date() < closeDeadline { await Task.yield() }
        #expect(closed.withLock { $0 })
    }

    @MainActor
    @Test(arguments: [false, true])
    func connectionChangeClosesFollowWithoutWaitingForAViewToDisappear(reconnect: Bool) async throws {
        let closed = Mutex(false)
        let (chunks, producer) = AsyncThrowingStream<[UInt8], Error>.makeStream()
        producer.onTermination = { _ in closed.withLock { $0 = true } }
        defer { producer.finish() }
        let client = RocketClient(config: testConfig(port: reconnect ? 42204 : 42205), streamOpener: { _ in
            producer.yield(Array(Self.tail.utf8))
            return JobLogStream.Connection(chunks: chunks, cancel: { producer.finish() })
        })
        // No file exists here: reconnect must fail locally, without opening a socket.
        let home = URL(fileURLWithPath: "/tmp/rkj-\(UUID().uuidString.prefix(8))")
        let controller = RocketController(home: home, client: client)
        let follow = Task { await controller.followJobLogs("j") }
        let deadline = Date().addingTimeInterval(1)
        while controller.store.jobLogs["j"]?.count != 3, Date() < deadline { await Task.yield() }
        #expect(controller.store.jobLogs["j"] == ["start", "repeat", "repeat"])
        if reconnect { await controller.connect(launchIfNeeded: false) } else { controller.disconnect() }
        producer.yield(Array("event: job.log\ndata: {\"type\":\"job.log\",\"job_id\":\"j\",\"line\":\"stale after connection change\"}\n\n".utf8))
        let closeDeadline = Date().addingTimeInterval(1)
        while !closed.withLock({ $0 }), Date() < closeDeadline { await Task.yield() }
        #expect(closed.withLock { $0 })
        #expect(controller.store.jobLogs["j"] == ["start", "repeat", "repeat"])
        #expect(controller.lastError == nil)
        // Finish our caller even on the initial RED implementation.
        follow.cancel()
        await follow.value
    }
}
