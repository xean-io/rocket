import Foundation
import Testing
@testable import RocketKit

@Suite("daemon.json")
struct DaemonConfigTests {
    private func tempDir() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rocketkit-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func write(_ data: Data, mode: Int16, in dir: URL) throws -> URL {
        let url = dir.appendingPathComponent("daemon.json")
        try data.write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: NSNumber(value: mode)], ofItemAtPath: url.path)
        return url
    }

    @Test func homeRespectsRocketHome() {
        let user = URL(fileURLWithPath: "/Users/me")
        #expect(DaemonConfigLoader.home(environment: ["ROCKET_HOME": "/tmp/rk"], userHome: user).path == "/tmp/rk")
        #expect(DaemonConfigLoader.home(environment: [:], userHome: user).path == "/Users/me/.rocket")
        #expect(DaemonConfigLoader.home(environment: ["ROCKET_HOME": ""], userHome: user).path == "/Users/me/.rocket")
        #expect(DaemonConfigLoader.configURL(home: URL(fileURLWithPath: "/tmp/rk")).path == "/tmp/rk/daemon.json")
    }

    @Test func parsesPrivateFileWithoutWarnings() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = try write(Fixture.data("daemon.json"), mode: 0o600, in: dir)
        let loaded = try DaemonConfigLoader.load(from: url)
        #expect(loaded.warnings.isEmpty)
        #expect(loaded.config.http.absoluteString == "http://127.0.0.1:52817")
        #expect(loaded.config.token == "s3cr3t")
        #expect(loaded.config.rocketBin == "/usr/local/bin/rocket")
        #expect(loaded.config.pid == 4242 && loaded.config.version == "0.1.0-dev" && loaded.config.api == "v1")
        #expect(loaded.config.startedAt != nil)
    }

    @Test func toleratesLoosePermissionsWithWarning() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = try write(Fixture.data("daemon.json"), mode: 0o644, in: dir)
        let loaded = try DaemonConfigLoader.load(from: url)
        #expect(loaded.config.token == "s3cr3t")
        #expect(loaded.warnings.count == 1)
        #expect(loaded.warnings[0].contains("0644"))
    }

    @Test func missingFile() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("daemon.json")
        #expect(throws: DaemonConfigError.missing(url.path)) { try DaemonConfigLoader.load(from: url) }
    }

    @Test func malformedOrIncompleteFile() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let bad = try write(Data("{not json".utf8), mode: 0o600, in: dir)
        #expect(throws: DaemonConfigError.self) { try DaemonConfigLoader.load(from: bad) }
        let noToken = try write(Data(#"{"http":"http://127.0.0.1:1"}"#.utf8), mode: 0o600, in: dir)
        #expect(throws: DaemonConfigError.self) { try DaemonConfigLoader.load(from: noToken) }
        let numericVersion = try write(Data(#"{"http":"http://127.0.0.1:1","token":"t","version":2}"#.utf8), mode: 0o600, in: dir)
        #expect(try DaemonConfigLoader.load(from: numericVersion).config.version == "2")
    }
}
