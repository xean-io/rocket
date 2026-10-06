import Foundation
import Testing
@testable import RocketKit

@Suite("RocketClient")
struct ClientTests {
    @Test func sendsBearerTokenAndDecodes() async throws {
        let port = 41001
        MockURLProtocol.register(port: port) { _ in (200, try! Fixture.data("health.json")) }
        let client = RocketClient(config: testConfig(port: port, token: "abc"), session: MockURLProtocol.session())
        let health = try await client.health()
        #expect(health.api == "v1")
        let req = try #require(MockURLProtocol.requests(port: port).first)
        #expect(req.value(forHTTPHeaderField: "Authorization") == "Bearer abc")
        #expect(req.url?.path == "/v1/health")
    }

    @Test func encodesQueryAndBody() async throws {
        let port = 41002
        MockURLProtocol.register(port: port) { req in
            req.url?.path == "/v1/up" ? (200, try! Fixture.data("up_result.json")) : (200, try! Fixture.data("ps.json"))
        }
        let client = RocketClient(config: testConfig(port: port), session: MockURLProtocol.session())
        _ = try await client.ps(project: "/code/my shop", all: false)
        _ = try await client.up(UpRequest(project: "nuvara", services: ["web"], env: "dev"))
        let reqs = MockURLProtocol.requests(port: port)
        #expect(reqs[0].url?.query == "project=/code/my%20shop")
        #expect(reqs[1].httpMethod == "POST")
        #expect(reqs[1].value(forHTTPHeaderField: "Content-Type") == "application/json")
        let body = try JSONSerialization.jsonObject(with: try #require(reqs[1].httpBody)) as? [String: Any]
        #expect(body?["owner"] as? String == "user" && (body?["services"] as? [String]) == ["web"])
    }

    @Test func mapsErrors() async throws {
        let port = 41003
        MockURLProtocol.register(port: port) { req in
            switch req.url?.path {
            case "/v1/health": return (401, Data("unauthorized".utf8))
            case "/v1/up": return (400, try! Fixture.data("error.json"))
            default: return (404, Data("404 page not found\n".utf8))
            }
        }
        let client = RocketClient(config: testConfig(port: port), session: MockURLProtocol.session())
        await #expect(throws: RocketError.unauthorized) { try await client.health() }
        await #expect(throws: RocketError.api(status: 400, code: "invalid",
                                              message: "invalid request: unknown service or group \"nope\" in project shop")) {
            try await client.up(UpRequest(project: "shop", services: ["nope"]))
        }
        await #expect(throws: RocketError.endpointUnavailable("/v1/jobs")) { try await client.jobs(project: nil) }
    }
}
