import Foundation
import Synchronization
@testable import RocketKit

enum Fixture {
    static func data(_ name: String) throws -> Data {
        let base = (name as NSString).deletingPathExtension
        let ext = (name as NSString).pathExtension
        guard let url = Bundle.module.url(forResource: base, withExtension: ext, subdirectory: "Fixtures") else {
            throw CocoaError(.fileNoSuchFile, userInfo: [NSFilePathErrorKey: name])
        }
        return try Data(contentsOf: url)
    }

    static func decode<T: Decodable>(_ type: T.Type, _ name: String) throws -> T {
        try RocketJSON.decoder.decode(T.self, from: data(name))
    }
}

/// URLProtocol stub. Handlers are keyed by URL port so parallel tests never share one.
final class MockURLProtocol: URLProtocol, @unchecked Sendable {
    typealias Handler = @Sendable (URLRequest) -> (Int, Data)
    private static let handlers = Mutex<[Int: Handler]>([:])
    private static let seen = Mutex<[Int: [URLRequest]]>([:])

    static func register(port: Int, _ handler: @escaping Handler) {
        handlers.withLock { $0[port] = handler }
    }

    static func requests(port: Int) -> [URLRequest] {
        seen.withLock { $0[port] ?? [] }
    }

    static func session() -> URLSession {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [MockURLProtocol.self]
        return URLSession(configuration: cfg)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let port = request.url?.port ?? 0
        var req = request
        if req.httpBody == nil, let stream = req.httpBodyStream {
            req.httpBody = Self.drain(stream)
        }
        let captured = req
        Self.seen.withLock { $0[port, default: []].append(captured) }
        guard let handler = Self.handlers.withLock({ $0[port] }) else {
            client?.urlProtocol(self, didFailWithError: URLError(.cannotConnectToHost))
            return
        }
        let (status, body) = handler(captured)
        let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: "HTTP/1.1",
                                       headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func drain(_ stream: InputStream) -> Data {
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buf = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable {
            let n = stream.read(&buf, maxLength: buf.count)
            if n <= 0 { break }
            data.append(buf, count: n)
        }
        return data
    }
}

func testConfig(port: Int, token: String = "tok") -> DaemonConfig {
    DaemonConfig(http: URL(string: "http://127.0.0.1:\(port)")!, token: token)
}
