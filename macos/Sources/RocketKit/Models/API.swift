import Foundation

/// The owner the app records on runs and jobs it starts (per the app contract).
public let appOwner = "user"

public struct HealthInfo: Codable, Hashable, Sendable {
    public var ok: Bool
    public var api: String
    public var version: String
    public var pid: Int
    public var startedAt: Date?
    public var home: String?
    public var socket: String?
}

public struct UpRequest: Codable, Hashable, Sendable {
    public var project: String
    public var services: [String]?
    public var env: String?
    public var owner: String?
    public var ttl: String?

    public init(project: String, services: [String]? = nil, env: String? = nil, owner: String? = appOwner,
                ttl: String? = nil) {
        self.project = project
        self.services = services
        self.env = env
        self.owner = owner
        self.ttl = ttl
    }
}

public struct ServiceResult: Codable, Hashable, Sendable {
    public var service: String
    public var action: String
    public var state: RunState
    public var health: Health?
    public var pid: Int?
    public var ports: [String: Int]?
    public var remaps: [PortRemap]?
    public var owner: String?
    public var expiresAt: Date?
    public var logPath: String?
    public var error: String?
}

public struct UpResult: Codable, Hashable, Sendable {
    public var project: String
    public var env: String
    public var services: [ServiceResult]

    /// Human summary of failed/skipped services, if any.
    public var failureSummary: String? {
        let bad = services.filter { $0.action == "failed" || $0.action == "skipped" }
        guard !bad.isEmpty else { return nil }
        return bad.map { "\($0.service): \($0.error ?? $0.action)" }.joined(separator: "\n")
    }
}

public struct DownRequest: Codable, Hashable, Sendable {
    public var project: String?
    public var services: [String]?
    public var owner: String?
    public var everywhere: Bool?

    public init(project: String?, services: [String]? = nil, owner: String? = nil, everywhere: Bool? = nil) {
        self.project = project
        self.services = services
        self.owner = owner
        self.everywhere = everywhere
    }
}

public struct DownResult: Codable, Hashable, Sendable {
    public var stopped: [Run]
    public var composeDown: [String]?
    /// Running jobs canceled by a whole-project, owner or everywhere down.
    public var canceledJobs: [String]?
    public var errors: [String]?

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        stopped = try c.decodeIfPresent([Run].self, forKey: .stopped) ?? []
        composeDown = try c.decodeIfPresent([String].self, forKey: .composeDown)
        canceledJobs = try c.decodeIfPresent([String].self, forKey: .canceledJobs)
        errors = try c.decodeIfPresent([String].self, forKey: .errors)
    }
}

public struct StatusResult: Codable, Hashable, Sendable {
    public var services: [Run]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        services = try c.decodeIfPresent([Run].self, forKey: .services) ?? []
    }
}

public struct LogsResult: Codable, Hashable, Sendable {
    public var project: String
    public var service: String
    public var lines: [String]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        project = try c.decode(String.self, forKey: .project)
        service = try c.decode(String.self, forKey: .service)
        lines = try c.decodeIfPresent([String].self, forKey: .lines) ?? []
    }
}

/// One entry of `GET /v1/ports` (`app.PortInfo`: a lease plus run details).
public struct PortInfo: Codable, Hashable, Sendable, Identifiable {
    public var port: Int
    public var project: String
    public var service: String
    public var portName: String
    public var createdAt: Date?
    public var owner: String?
    public var state: RunState?
    public var pid: Int?
    public var env: String?

    public var id: Int { port }

    public init(lease: Lease) {
        port = lease.port
        project = lease.project
        service = lease.service
        portName = lease.portName
        createdAt = lease.createdAt
    }
}

public struct PortsResult: Codable, Hashable, Sendable {
    public var ports: [PortInfo]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        ports = try c.decodeIfPresent([PortInfo].self, forKey: .ports) ?? []
    }
}

public struct GCAction: Codable, Hashable, Sendable {
    public var action: String
    public var project: String
    public var service: String
    public var port: Int?
    public var detail: String?
}

public struct GCResult: Codable, Hashable, Sendable {
    public var actions: [GCAction]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        actions = try c.decodeIfPresent([GCAction].self, forKey: .actions) ?? []
    }
}

public struct ProjectRef: Codable, Hashable, Sendable, Identifiable {
    public var name: String
    public var path: String
    public var addedAt: Date?

    public var id: String { name }
}

public struct ProjectsResult: Codable, Hashable, Sendable {
    public var projects: [ProjectRef]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        projects = try c.decodeIfPresent([ProjectRef].self, forKey: .projects) ?? []
    }
}

/// Body of every non-2xx daemon response.
public struct APIErrorBody: Codable, Hashable, Sendable {
    public var error: String
    public var code: String?
}

/// Event types published on `GET /v1/events`.
public struct EventType: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let serviceState = EventType(rawValue: "service.state")
    public static let logLine = EventType(rawValue: "log.line")
    public static let portLeased = EventType(rawValue: "port.leased")
    public static let portReleased = EventType(rawValue: "port.released")
    public static let jobState = EventType(rawValue: "job.state")
    public static let jobLog = EventType(rawValue: "job.log")
}

/// SSE payload (`domain.Event`).
public struct DaemonEvent: Codable, Hashable, Sendable {
    public var type: EventType
    public var time: Date?
    public var project: String?
    public var service: String?
    public var state: RunState?
    public var line: String?
    public var run: Run?
    public var lease: Lease?
    public var jobId: String?
    public var status: JobStatus?
    public var job: Job?
}

/// Project header of a `Summary` (`app.ProjectInfo`).
public struct ProjectInfo: Codable, Hashable, Sendable {
    public var name: String
    public var root: String?
    public var defaultEnv: String?
    /// Nil for older daemons; an empty list is authoritative.
    public var envs: [String]?
    public var pipelines: [String]?
    public var deployEnvs: [String]?
}

/// A port problem reported by `GET /v1/status` (`app.Conflict`).
public struct Conflict: Codable, Hashable, Sendable, Identifiable {
    public var kind: String
    public var project: String
    public var service: String
    public var portName: String
    public var port: Int
    public var `default`: Int
    public var env: String?
    public var remappable: Bool?
    public var holder: PortHolder?
    public var detail: String?

    public var id: String { "\(kind):\(project)/\(service):\(portName)" }
    public var isRemap: Bool { kind == "port_remapped" }
}

/// `GET /v1/status?project=`: declared services, running jobs and conflicts.
public struct Summary: Codable, Hashable, Sendable {
    public var project: ProjectInfo?
    public var services: [Run]
    public var jobs: [Job]
    public var conflicts: [Conflict]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        project = try c.decodeIfPresent(ProjectInfo.self, forKey: .project)
        services = try c.decodeIfPresent([Run].self, forKey: .services) ?? []
        jobs = try c.decodeIfPresent([Job].self, forKey: .jobs) ?? []
        conflicts = try c.decodeIfPresent([Conflict].self, forKey: .conflicts) ?? []
    }
}

/// `GET /v1/jobs/{id}/logs`.
public struct JobLogsResult: Codable, Hashable, Sendable {
    public var job: String
    public var project: String?
    public var lines: [String]

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        job = try c.decodeIfPresent(String.self, forKey: .job) ?? ""
        project = try c.decodeIfPresent(String.self, forKey: .project)
        lines = try c.decodeIfPresent([String].self, forKey: .lines) ?? []
    }
}
