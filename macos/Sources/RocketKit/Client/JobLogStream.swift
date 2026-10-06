import Foundation

/// One ordered tail/follow subscription. Completion never replays the tail.
public struct JobLogStream: Sendable {
    /// The HTTP body and explicit teardown for terminal and cancellation paths.
    public struct Connection: Sendable {
        public let chunks: AsyncThrowingStream<[UInt8], Error>
        private let cancel: @Sendable () -> Void

        public init(chunks: AsyncThrowingStream<[UInt8], Error>, cancel: @escaping @Sendable () -> Void) {
            self.chunks = chunks
            self.cancel = cancel
        }

        public func close() { cancel() }
    }

    public typealias Connect = @Sendable () async throws -> Connection
    private let connect: Connect

    public init(connect: @escaping Connect) { self.connect = connect }
    public func events() -> AsyncThrowingStream<DaemonEvent, Error> {
        let connect = self.connect
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let connection = try await connect()
                    defer { connection.close() }
                    var parser = SSEParser()
                    for try await chunk in connection.chunks {
                        try Task.checkCancellation()
                        for frame in parser.feed(chunk) {
                            guard case .message(let message) = frame else { continue }
                            let event = try RocketJSON.decoder.decode(DaemonEvent.self, from: Data(message.data.utf8))
                            guard event.type == .jobLog || event.type == .jobState else { continue }
                            continuation.yield(event)
                            if event.type == .jobState, (event.status ?? event.job?.status)?.isTerminal == true {
                                continuation.finish()
                                return
                            }
                        }
                    }
                    try Task.checkCancellation()
                    throw RocketError.badResponse("job log stream closed before its final state")
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
