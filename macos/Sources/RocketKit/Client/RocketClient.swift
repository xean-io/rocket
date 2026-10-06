import Foundation

public enum RocketError: Error, Equatable, Sendable, LocalizedError {
    case unauthorized
    case api(status: Int, code: String?, message: String)
    /// The route does not exist on this daemon (older rocketd).
    case endpointUnavailable(String)
    case badResponse(String)
    case transport(String)

    public var errorDescription: String? {
        switch self {
        case .unauthorized: "The daemon rejected the token in daemon.json."
        case .api(_, _, let message): message
        case .endpointUnavailable(let path): "This rocketd does not support \(path) yet."
        case .badResponse(let why): "Unexpected daemon response: \(why)"
        case .transport(let why): why
        }
    }
}

/// HTTP client for rocketd's TCP listener (bearer token from daemon.json).
public actor RocketClient {
    public typealias StreamOpener = @Sendable (URLRequest) async throws -> JobLogStream.Connection
    public nonisolated let config: DaemonConfig
    private let session: URLSession
    private let streamOpener: StreamOpener?

    public init(config: DaemonConfig, session: URLSession = .shared, streamOpener: StreamOpener? = nil) {
        self.config = config
        self.session = session
        self.streamOpener = streamOpener
    }

    // MARK: Endpoints

    public func health() async throws -> HealthInfo { try await get("/v1/health") }

    public func up(_ req: UpRequest) async throws -> UpResult { try await post("/v1/up", req) }
    public func restart(_ req: UpRequest) async throws -> UpResult { try await post("/v1/restart", req) }
    public func down(_ req: DownRequest) async throws -> DownResult { try await post("/v1/down", req) }
    public func gc() async throws -> GCResult { try await post("/v1/gc", Empty?.none) }

    public func ps(project: String? = nil, all: Bool = false) async throws -> StatusResult {
        var q: [URLQueryItem] = []
        if let project { q.append(URLQueryItem(name: "project", value: project)) }
        if all { q.append(URLQueryItem(name: "all", value: "true")) }
        return try await get("/v1/ps", query: q)
    }

    public func logs(project: String, service: String, tail: Int = 200) async throws -> LogsResult {
        try await get("/v1/logs", query: [
            URLQueryItem(name: "project", value: project),
            URLQueryItem(name: "service", value: service),
            URLQueryItem(name: "tail", value: String(tail)),
        ])
    }

    public func ports() async throws -> PortsResult { try await get("/v1/ports") }
    public func projects() async throws -> ProjectsResult { try await get("/v1/projects") }

    public func addProject(path: String) async throws -> ProjectRef {
        try await post("/v1/projects", ["path": path])
    }

    public func removeProject(name: String) async throws {
        let escaped = name.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? name
        let _: [String: String] = try await send("DELETE", "/v1/projects/\(escaped)", query: [], body: nil)
    }

    /// One project's declared services, running jobs and port conflicts.
    public func status(project: String) async throws -> Summary {
        try await get("/v1/status", query: [URLQueryItem(name: "project", value: project)])
    }

    public func jobs(project: String?, limit: Int = 100) async throws -> JobsResult {
        var q = [URLQueryItem(name: "limit", value: String(limit))]
        if let project {
            q.append(URLQueryItem(name: "project", value: project))
        } else {
            q.append(URLQueryItem(name: "all", value: "true"))
        }
        return try await get("/v1/jobs", query: q)
    }

    public func job(id: String) async throws -> Job { try await get("/v1/jobs/\(id)") }
    public func startJob(_ req: JobRequest) async throws -> Job { try await post("/v1/jobs", req) }
    public func cancelJob(id: String) async throws -> Job { try await post("/v1/jobs/\(id)/cancel", Empty?.none) }

    public func jobLogs(id: String, tail: Int = 200) async throws -> JobLogsResult {
        try await get("/v1/jobs/\(id)/logs", query: [URLQueryItem(name: "tail", value: String(tail))])
    }

    /// The daemon drains the tail/live file before sending one final job state.
    public nonisolated func jobLogStream(id: String, tail: Int = 300) -> JobLogStream {
        let request = makeRequest("GET", "/v1/jobs/\(id)/logs", query: [
            URLQueryItem(name: "follow", value: "true"),
            URLQueryItem(name: "tail", value: String(tail)),
        ], body: nil, stream: true)
        let session = self.session
        let opener = self.streamOpener
        return JobLogStream(connect: {
            if let opener { return try await opener(request) }
            return try await Self.openStreamConnection(request, session: session)
        })
    }

    /// A reconnecting subscription to `GET /v1/events`.
    public nonisolated func events(project: String? = nil, service: String? = nil,
                                   types: [EventType] = []) -> EventStream {
        var q: [URLQueryItem] = []
        if let project { q.append(URLQueryItem(name: "project", value: project)) }
        if let service { q.append(URLQueryItem(name: "service", value: service)) }
        if !types.isEmpty { q.append(URLQueryItem(name: "types", value: types.map(\.rawValue).joined(separator: ","))) }
        let request = makeRequest("GET", "/v1/events", query: q, body: nil, stream: true)
        let session = self.session
        return EventStream(connect: { try await Self.openStream(request, session: session) })
    }

    // MARK: Plumbing

    private struct Empty: Codable {}

    private func get<T: Decodable>(_ path: String, query: [URLQueryItem] = []) async throws -> T {
        try await send("GET", path, query: query, body: nil)
    }

    private func post<B: Encodable, T: Decodable>(_ path: String, _ body: B?) async throws -> T {
        let data = try body.map { try RocketJSON.encoder.encode($0) }
        return try await send("POST", path, query: [], body: data)
    }

    private func send<T: Decodable>(_ method: String, _ path: String, query: [URLQueryItem], body: Data?) async throws -> T {
        let request = makeRequest(method, path, query: query, body: body, stream: false)
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw RocketError.transport(error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else { throw RocketError.badResponse("not HTTP") }
        try Self.check(status: http.statusCode, body: data, path: path)
        do {
            return try RocketJSON.decoder.decode(T.self, from: data)
        } catch {
            throw RocketError.badResponse("\(path): \(error)")
        }
    }

    private nonisolated func makeRequest(_ method: String, _ path: String, query: [URLQueryItem], body: Data?,
                                         stream: Bool) -> URLRequest {
        var comps = URLComponents(url: config.http, resolvingAgainstBaseURL: false)!
        comps.path = path
        comps.queryItems = query.isEmpty ? nil : query
        var req = URLRequest(url: comps.url!)
        req.httpMethod = method
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        req.setValue(stream ? "text/event-stream" : "application/json", forHTTPHeaderField: "Accept")
        if let body {
            req.httpBody = body
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        // up/down block until services settle; streams stay open indefinitely.
        req.timeoutInterval = stream ? 60 * 60 * 24 : 180
        return req
    }

    static func check(status: Int, body: Data, path: String) throws {
        guard !(200..<300).contains(status) else { return }
        if status == 401 { throw RocketError.unauthorized }
        if let err = try? RocketJSON.decoder.decode(APIErrorBody.self, from: body) {
            throw RocketError.api(status: status, code: err.code, message: err.error)
        }
        if status == 404 || status == 405 { throw RocketError.endpointUnavailable(path) }
        let text = String(decoding: body.prefix(200), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        throw RocketError.api(status: status, code: nil, message: text.isEmpty ? "HTTP \(status)" : text)
    }

    private static func openStream(_ request: URLRequest, session: URLSession) async throws
        -> AsyncThrowingStream<[UInt8], Error> {
        try await openStreamConnection(request, session: session).chunks
    }

    private static func openStreamConnection(_ request: URLRequest, session: URLSession) async throws
        -> JobLogStream.Connection {
        let (bytes, response) = try await session.bytes(for: request)
        let dataTask = bytes.task
        guard let http = response as? HTTPURLResponse else {
            dataTask.cancel()
            throw RocketError.badResponse("not HTTP")
        }
        if !(200..<300).contains(http.statusCode) {
            dataTask.cancel()
            throw RocketError.api(status: http.statusCode, code: nil, message: "event stream HTTP \(http.statusCode)")
        }
        let (chunks, continuation) = AsyncThrowingStream<[UInt8], Error>.makeStream()
        let task = Task {
            do {
                var chunk: [UInt8] = []
                chunk.reserveCapacity(1024)
                for try await byte in bytes {
                    chunk.append(byte)
                    if byte == UInt8(ascii: "\n") {
                        continuation.yield(chunk)
                        chunk.removeAll(keepingCapacity: true)
                    }
                }
                if !chunk.isEmpty { continuation.yield(chunk) }
                continuation.finish()
            } catch {
                continuation.finish(throwing: error)
            }
        }
        continuation.onTermination = { _ in task.cancel(); dataTask.cancel() }
        return JobLogStream.Connection(chunks: chunks, cancel: {
            task.cancel()
            dataTask.cancel()
            continuation.finish()
        })
    }
}
