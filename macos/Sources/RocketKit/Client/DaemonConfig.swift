import Foundation

/// Contents of `$ROCKET_HOME/daemon.json`, written by rocketd for TCP clients.
public struct DaemonConfig: Codable, Hashable, Sendable {
    public var version: String?
    public var api: String?
    public var pid: Int?
    public var socket: String?
    public var http: URL
    public var token: String
    public var rocketBin: String?
    public var startedAt: Date?

    public init(http: URL, token: String, version: String? = nil, pid: Int? = nil, socket: String? = nil,
                rocketBin: String? = nil, startedAt: Date? = nil) {
        self.http = http
        self.token = token
        self.version = version
        self.pid = pid
        self.socket = socket
        self.rocketBin = rocketBin
        self.startedAt = startedAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let rawURL = try c.decode(String.self, forKey: .http)
        guard let url = URL(string: rawURL), url.scheme == "http" || url.scheme == "https", url.host != nil else {
            throw DecodingError.dataCorruptedError(forKey: .http, in: c, debugDescription: "invalid http url \(rawURL)")
        }
        http = url
        token = try c.decode(String.self, forKey: .token)
        if let s = try? c.decodeIfPresent(String.self, forKey: .version) {
            version = s
        } else if let n = try? c.decodeIfPresent(Int.self, forKey: .version) {
            version = String(n)
        }
        api = try? c.decodeIfPresent(String.self, forKey: .api)
        pid = try? c.decodeIfPresent(Int.self, forKey: .pid)
        socket = try? c.decodeIfPresent(String.self, forKey: .socket)
        rocketBin = try? c.decodeIfPresent(String.self, forKey: .rocketBin)
        startedAt = try? c.decodeIfPresent(Date.self, forKey: .startedAt)
    }
}

public enum DaemonConfigError: Error, Equatable, Sendable, LocalizedError {
    case missing(String)
    case unreadable(String, String)
    case malformed(String)

    public var errorDescription: String? {
        switch self {
        case .missing(let path): "No daemon.json at \(path)"
        case .unreadable(let path, let why): "Cannot read \(path): \(why)"
        case .malformed(let why): "daemon.json is malformed: \(why)"
        }
    }
}

public struct LoadedDaemonConfig: Sendable, Equatable {
    public var config: DaemonConfig
    /// Non-fatal problems, e.g. permissions looser than 0600.
    public var warnings: [String]
}

public enum DaemonConfigLoader {
    /// `$ROCKET_HOME` when set and non-empty, else `~/.rocket`.
    public static func home(environment: [String: String] = ProcessInfo.processInfo.environment,
                            userHome: URL = FileManager.default.homeDirectoryForCurrentUser) -> URL {
        if let home = environment["ROCKET_HOME"], !home.isEmpty {
            return URL(fileURLWithPath: (home as NSString).expandingTildeInPath).standardizedFileURL
        }
        return userHome.appendingPathComponent(".rocket")
    }

    public static func configURL(home: URL) -> URL {
        home.appendingPathComponent("daemon.json")
    }

    public static func load(from url: URL) throws(DaemonConfigError) -> LoadedDaemonConfig {
        let fm = FileManager.default
        guard fm.fileExists(atPath: url.path) else { throw .missing(url.path) }
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            throw .unreadable(url.path, error.localizedDescription)
        }
        let config: DaemonConfig
        do {
            config = try RocketJSON.decoder.decode(DaemonConfig.self, from: data)
        } catch {
            throw .malformed(String(describing: error))
        }
        var warnings: [String] = []
        if let perms = (try? fm.attributesOfItem(atPath: url.path))?[.posixPermissions] as? NSNumber {
            let mode = perms.intValue & 0o777
            if mode & 0o077 != 0 {
                warnings.append(String(format: "%@ has mode %04o; expected 0600 (the token is readable by others)",
                                       url.path, mode))
            }
        }
        return LoadedDaemonConfig(config: config, warnings: warnings)
    }
}
