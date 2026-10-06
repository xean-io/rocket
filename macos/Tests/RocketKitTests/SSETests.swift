import Foundation
import Synchronization
import Testing
@testable import RocketKit

@Suite("SSE parser")
struct SSEParserTests {
    private func parse(_ chunks: [String]) -> [SSEFrame] {
        var p = SSEParser()
        return chunks.flatMap { p.feed(Array($0.utf8)) }
    }

    @Test func dispatchesOnBlankLineWithEventName() {
        let frames = parse(["event: service.state\ndata: {\"a\":1}\n\n"])
        #expect(frames == [.message(SSEMessage(event: "service.state", data: "{\"a\":1}"))])
    }

    @Test func joinsMultiLineDataAndDefaultsEventName() {
        let frames = parse(["data: one\ndata:two\ndata:  three\n\n"])
        #expect(frames == [.message(SSEMessage(event: "message", data: "one\ntwo\n three"))])
    }

    @Test func pingCommentsAreReported() {
        let frames = parse([": ping\n\n"])
        #expect(frames == [.comment("ping")])
    }

    @Test func handlesChunksSplitMidLineAndCRLF() {
        let frames = parse(["eve", "nt: log.line\r", "\ndata: hel", "lo\r\n", "\r\n", "data: x\n"])
        #expect(frames == [.message(SSEMessage(event: "log.line", data: "hello"))])
    }

    @Test func idRetryAndEmptyDataBlocks() {
        let frames = parse(["id: 7\nretry: 2500\nevent: x\n\n", "event: y\ndata: z\n\n"])
        // A block without data dispatches nothing but the event name must reset.
        #expect(frames == [.retry(2500), .message(SSEMessage(event: "y", data: "z", id: "7"))])
    }

    @Test func fixtureStream() throws {
        var p = SSEParser()
        let frames = p.feed(Array(try Fixture.data("events.sse")))
        #expect(frames.count == 3)
        guard case .message(let first) = frames[0], case .comment = frames[1], case .message(let third) = frames[2] else {
            Issue.record("unexpected frames \(frames)")
            return
        }
        let ev = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(first.data.utf8))
        #expect(ev.type == .serviceState && ev.run?.pid == 8635 && ev.state == .running)
        let log = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(third.data.utf8))
        #expect(log.type == .logLine && log.line == "listening on :3002")
    }
}

/// Scripted connections: each call to `connect` consumes the next script entry.
private actor Script {
    enum Step { case chunks([String]), fail, hang }
    private var steps: [Step]
    private(set) var calls = 0
    init(_ steps: [Step]) { self.steps = steps }
    func next() -> Step {
        calls += 1
        return steps.isEmpty ? .hang : steps.removeFirst()
    }
}

@Suite("Event stream reconnect")
struct EventStreamTests {
    private static let state = #"{"type":"service.state","project":"p","service":"s","state":"running"}"#
    private static let log = #"{"type":"log.line","project":"p","service":"s","line":"hi"}"#

    @Test func reconnectsWithBackoffAndResetsAfterSuccess() async throws {
        let script = Script([
            .chunks(["event: service.state\ndata: \(Self.state)\n\n"]),
            .fail,
            .fail,
            .chunks([": ping\n\n", "event: log.line\ndata: \(Self.log)\n\n"]),
        ])
        let delays = Mutex<[Duration]>([])
        let stream = EventStream(
            connect: {
                switch await script.next() {
                case .fail:
                    throw URLError(.cannotConnectToHost)
                case .hang:
                    return AsyncThrowingStream { _ in }
                case .chunks(let chunks):
                    return AsyncThrowingStream { c in
                        for chunk in chunks { c.yield(Array(chunk.utf8)) }
                        c.finish()
                    }
                }
            },
            backoff: Backoff(initial: .milliseconds(100), maximum: .milliseconds(300)),
            sleep: { d in delays.withLock { $0.append(d) } }
        )

        var updates: [String] = []
        var events = 0
        for await update in stream.updates() {
            switch update {
            case .connected: updates.append("connected")
            case .ping: updates.append("ping")
            case .disconnected: updates.append("disconnected")
            case .event(let e):
                updates.append(e.type.rawValue)
                events += 1
            }
            if events == 2 { break }
        }
        #expect(updates == [
            "connected", "service.state", "disconnected",
            "disconnected", "disconnected",
            "connected", "ping", "log.line",
        ])
        // first drop after a good connection waits the initial delay; failures double it.
        #expect(Array(delays.withLock { $0 }.prefix(3)) == [.milliseconds(100), .milliseconds(200), .milliseconds(300)])
        #expect(await script.calls >= 4) // the last stream ends, so a 5th attempt may already be underway
    }

    @Test func malformedEventDataIsSkipped() async {
        let stream = EventStream(
            connect: {
                AsyncThrowingStream { c in
                    c.yield(Array("event: service.state\ndata: {oops\n\nevent: log.line\ndata: \(Self.log)\n\n".utf8))
                }
            },
            backoff: .default,
            sleep: { _ in }
        )
        for await update in stream.updates() {
            if case .event(let e) = update {
                #expect(e.type == .logLine)
                break
            }
        }
    }

    @Test func backoffDoublesAndCaps() {
        let b = Backoff(initial: .milliseconds(500), maximum: .seconds(10))
        #expect(b.delay(attempt: 0) == .milliseconds(500))
        #expect(b.delay(attempt: 1) == .seconds(1))
        #expect(b.delay(attempt: 3) == .seconds(4))
        #expect(b.delay(attempt: 10) == .seconds(10))
    }
}
