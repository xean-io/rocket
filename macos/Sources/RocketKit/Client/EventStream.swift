import Foundation

/// Exponential reconnect delay.
public struct Backoff: Sendable, Hashable {
    public var initial: Duration
    public var maximum: Duration

    public init(initial: Duration, maximum: Duration) {
        self.initial = initial
        self.maximum = maximum
    }

    public static let `default` = Backoff(initial: .milliseconds(500), maximum: .seconds(10))

    /// Delay before reconnect attempt `attempt` (0-based): initial * 2^attempt, capped.
    public func delay(attempt: Int) -> Duration {
        var d = initial
        for _ in 0..<min(attempt, 30) {
            d *= 2
            if d >= maximum { return maximum }
        }
        return min(d, maximum)
    }
}

public enum StreamUpdate: Sendable {
    case connected
    case event(DaemonEvent)
    case ping
    /// The connection dropped or could not be opened; a reconnect follows.
    case disconnected(String)
}

/// A self-healing SSE subscription. `connect` opens one HTTP stream and
/// returns its body as byte chunks; the stream reconnects with backoff
/// until the consumer stops iterating.
public struct EventStream: Sendable {
    public typealias Connect = @Sendable () async throws -> AsyncThrowingStream<[UInt8], Error>
    public typealias Sleep = @Sendable (Duration) async throws -> Void

    private let connect: Connect
    private let backoff: Backoff
    private let sleep: Sleep

    public init(connect: @escaping Connect, backoff: Backoff = .default,
                sleep: @escaping Sleep = { try await Task.sleep(for: $0) }) {
        self.connect = connect
        self.backoff = backoff
        self.sleep = sleep
    }

    public func updates() -> AsyncStream<StreamUpdate> {
        let connect = self.connect
        let backoff = self.backoff
        let sleep = self.sleep
        return AsyncStream { continuation in
            let task = Task {
                var attempt = 0
                while !Task.isCancelled {
                    var reason = "stream closed"
                    do {
                        let chunks = try await connect()
                        attempt = 0
                        continuation.yield(.connected)
                        var parser = SSEParser()
                        for try await chunk in chunks {
                            for frame in parser.feed(chunk) {
                                switch frame {
                                case .comment:
                                    continuation.yield(.ping)
                                case .message(let m):
                                    if let ev = try? RocketJSON.decoder.decode(DaemonEvent.self, from: Data(m.data.utf8)) {
                                        continuation.yield(.event(ev))
                                    }
                                case .retry:
                                    break
                                }
                            }
                        }
                    } catch is CancellationError {
                        break
                    } catch {
                        reason = error.localizedDescription
                    }
                    if Task.isCancelled { break }
                    continuation.yield(.disconnected(reason))
                    do {
                        try await sleep(backoff.delay(attempt: attempt))
                    } catch {
                        break
                    }
                    attempt += 1
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
