import Foundation

public struct JobStatus: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let running = JobStatus(rawValue: "running")
    public static let succeeded = JobStatus(rawValue: "succeeded")
    public static let failed = JobStatus(rawValue: "failed")
    public static let canceled = JobStatus(rawValue: "canceled")
    public static let lost = JobStatus(rawValue: "lost")

    public var isTerminal: Bool { self != .running }
}

public struct JobKind: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let setup = JobKind(rawValue: "setup")
    public static let pipeline = JobKind(rawValue: "pipeline")
    public static let deploy = JobKind(rawValue: "deploy")
}

/// One pipeline/setup step (`domain.Step`): exactly one of task or run.
public struct Step: Codable, Hashable, Sendable {
    public var task: String?
    public var run: String?

    public var describe: String {
        if let task, !task.isEmpty { return "task \(task)" }
        return "run \(run ?? "")"
    }
}

/// A supervised pipeline, setup action or deploy (`domain.Job`).
public struct Job: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var project: String
    public var name: String
    public var kind: JobKind
    public var env: String?
    public var profiles: [String]?
    public var owner: String?
    public var steps: [Step]
    public var args: [String]?
    public var status: JobStatus
    public var step: Int?
    public var pid: Int?
    public var pgid: Int?
    public var exitCode: Int?
    public var startedAt: Date?
    public var finishedAt: Date?
    public var expiresAt: Date?
    public var durationMs: Int?
    public var logPath: String?
    public var error: String?

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        project = try c.decodeIfPresent(String.self, forKey: .project) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        kind = try c.decodeIfPresent(JobKind.self, forKey: .kind) ?? .pipeline
        env = try c.decodeIfPresent(String.self, forKey: .env)
        profiles = try c.decodeIfPresent([String].self, forKey: .profiles)
        owner = try c.decodeIfPresent(String.self, forKey: .owner)
        steps = try c.decodeIfPresent([Step].self, forKey: .steps) ?? []
        args = try c.decodeIfPresent([String].self, forKey: .args)
        status = try c.decodeIfPresent(JobStatus.self, forKey: .status) ?? .running
        step = try c.decodeIfPresent(Int.self, forKey: .step)
        pid = try c.decodeIfPresent(Int.self, forKey: .pid)
        pgid = try c.decodeIfPresent(Int.self, forKey: .pgid)
        exitCode = try c.decodeIfPresent(Int.self, forKey: .exitCode)
        startedAt = try c.decodeIfPresent(Date.self, forKey: .startedAt)
        finishedAt = try c.decodeIfPresent(Date.self, forKey: .finishedAt)
        expiresAt = try c.decodeIfPresent(Date.self, forKey: .expiresAt)
        durationMs = try c.decodeIfPresent(Int.self, forKey: .durationMs)
        logPath = try c.decodeIfPresent(String.self, forKey: .logPath)
        error = try c.decodeIfPresent(String.self, forKey: .error)
    }
}

/// Body of `POST /v1/jobs`, mirroring `app.JobRequest`.
public struct JobRequest: Codable, Hashable, Sendable {
    public var project: String
    public var kind: JobKind
    public var name: String
    public var args: [String]?
    public var env: String?
    public var profiles: [String]?
    public var ttl: String?
    public var owner: String?
    public var yes: Bool?
    public var allowAgentDeploy: Bool?

    public init(project: String, kind: JobKind, name: String, args: [String]? = nil,
                env: String? = nil, profiles: [String]? = nil, ttl: String? = nil, owner: String? = appOwner,
                yes: Bool? = nil, allowAgentDeploy: Bool? = nil) {
        self.project = project
        self.kind = kind
        self.name = name
        self.args = args
        self.env = env
        self.profiles = profiles
        self.ttl = ttl
        self.owner = owner
        self.yes = yes
        self.allowAgentDeploy = allowAgentDeploy
    }

    /// Re-run request for a finished job.
    public init(rerunning job: Job, owner: String? = appOwner) {
        let isDeploy = job.kind == .deploy
        let env = isDeploy ? job.env ?? job.name : job.env
        self.init(project: job.project, kind: job.kind, name: isDeploy ? env ?? job.name : job.name,
                  args: job.args, env: env, profiles: isDeploy ? nil : job.profiles, owner: owner)
    }
}

public struct JobsResult: Codable, Hashable, Sendable {
    public var jobs: [Job]
}
