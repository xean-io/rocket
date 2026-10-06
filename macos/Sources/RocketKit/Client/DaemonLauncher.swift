import Foundation

/// Finds the `rocket` CLI and asks it to start rocketd.
public enum DaemonLauncher {
    public enum LaunchError: Error, LocalizedError, Sendable {
        case notFound
        case failed(Int32, String)
        case timedOut

        public var errorDescription: String? {
            switch self {
            case .notFound: "The rocket CLI was not found. Install it or put it on your PATH."
            case .failed(let code, let out): "rocket daemon start exited with \(code): \(out)"
            case .timedOut: "rocket daemon start did not finish in time."
            }
        }
    }

    /// GUI apps get a minimal PATH, so common install locations are added.
    public static func locateRocket(config: DaemonConfig?,
                                    environment: [String: String] = ProcessInfo.processInfo.environment) -> URL? {
        let fm = FileManager.default
        var candidates: [String] = []
        if let bin = config?.rocketBin { candidates.append(bin) }
        if let bin = environment["ROCKET_BIN"] { candidates.append(bin) }
        let home = fm.homeDirectoryForCurrentUser.path
        let path = (environment["PATH"] ?? "").split(separator: ":").map(String.init)
        let extra = ["/opt/homebrew/bin", "/usr/local/bin", "\(home)/go/bin", "\(home)/.local/bin", "\(home)/bin"]
        candidates += (path + extra).map { ($0 as NSString).appendingPathComponent("rocket") }
        return candidates.lazy.first { fm.isExecutableFile(atPath: $0) }.map { URL(fileURLWithPath: $0) }
    }

    /// Runs `rocket daemon start` (which detaches rocketd) and waits for it to return.
    public static func startDaemon(rocket: URL, home: URL, timeout: Duration = .seconds(30)) async throws {
        let process = Process()
        process.executableURL = rocket
        process.arguments = ["daemon", "start"]
        var env = ProcessInfo.processInfo.environment
        env["ROCKET_HOME"] = home.path
        process.environment = env
        // A file, not a pipe: the detached rocketd may inherit the descriptor.
        let outURL = FileManager.default.temporaryDirectory.appendingPathComponent("rocket-start-\(UUID().uuidString).log")
        FileManager.default.createFile(atPath: outURL.path, contents: nil)
        defer { try? FileManager.default.removeItem(at: outURL) }
        let out = try FileHandle(forWritingTo: outURL)
        defer { try? out.close() }
        process.standardOutput = out
        process.standardError = out
        process.standardInput = FileHandle.nullDevice
        let proc = process

        let status: Int32 = try await withThrowingTaskGroup(of: Int32.self) { group in
            group.addTask {
                try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Int32, Error>) in
                    proc.terminationHandler = { cont.resume(returning: $0.terminationStatus) }
                    do { try proc.run() } catch { cont.resume(throwing: error) }
                }
            }
            group.addTask {
                try await Task.sleep(for: timeout)
                proc.terminate()
                throw LaunchError.timedOut
            }
            defer { group.cancelAll() }
            return try await group.next()!
        }
        if status != 0 {
            let text = (try? String(contentsOf: outURL, encoding: .utf8)) ?? ""
            throw LaunchError.failed(status, String(text.prefix(500)).trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    /// Runs `rocket daemon stop`.
    public static func stopDaemon(rocket: URL, home: URL) async throws {
        let process = Process()
        process.executableURL = rocket
        process.arguments = ["daemon", "stop"]
        var env = ProcessInfo.processInfo.environment
        env["ROCKET_HOME"] = home.path
        process.environment = env
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        _ = try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Int32, Error>) in
            process.terminationHandler = { cont.resume(returning: $0.terminationStatus) }
            do { try process.run() } catch { cont.resume(throwing: error) }
        }
    }
}
