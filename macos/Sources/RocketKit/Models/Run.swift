import Foundation

/// Lifecycle state of a service run (`domain.RunState`).
public struct RunState: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let starting = RunState(rawValue: "starting")
    public static let running = RunState(rawValue: "running")
    public static let stopping = RunState(rawValue: "stopping")
    public static let stopped = RunState(rawValue: "stopped")
    public static let exited = RunState(rawValue: "exited")
    public static let failed = RunState(rawValue: "failed")
    public static let dead = RunState(rawValue: "dead")

    /// Mirrors `RunState.Active()`: the run may still own processes or ports.
    public var isActive: Bool { self == .starting || self == .running || self == .stopping }
}

/// Readiness of a run (`domain.Health`).
public struct Health: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let unknown = Health(rawValue: "unknown")
    public static let healthy = Health(rawValue: "healthy")
    public static let unhealthy = Health(rawValue: "unhealthy")
}

/// Which adapter supervises a service (`domain.ServiceKind`).
public struct ServiceKind: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let run = ServiceKind(rawValue: "run")
    public static let task = ServiceKind(rawValue: "task")
    public static let compose = ServiceKind(rawValue: "compose")
}

/// Identity of a service across projects.
public struct ServiceKey: Hashable, Sendable, Comparable, CustomStringConvertible {
    public let project: String
    public let service: String

    public init(project: String, service: String) {
        self.project = project
        self.service = service
    }

    public var description: String { "\(project)/\(service)" }

    public static func < (a: ServiceKey, b: ServiceKey) -> Bool {
        (a.project, a.service) < (b.project, b.service)
    }
}

/// Current or last instance of a service (`domain.Run`). Zero values are
/// omitted on the wire, hence the optionals.
public struct Run: Codable, Hashable, Sendable, Identifiable {
    public var project: String
    public var service: String
    public var env: String?
    public var kind: ServiceKind?
    public var state: RunState
    public var health: Health?
    public var pid: Int?
    public var pgid: Int?
    public var composeProject: String?
    public var containerId: String?
    public var owner: String?
    public var expiresAt: Date?
    public var startedAt: Date?
    public var stoppedAt: Date?
    public var exitCode: Int?
    public var ports: [String: Int]?
    public var logPath: String?
    public var error: String?

    public init(project: String, service: String, state: RunState) {
        self.project = project
        self.service = service
        self.state = state
    }

    public var id: ServiceKey { ServiceKey(project: project, service: service) }

    /// Ports sorted by name, for stable display.
    public var sortedPorts: [(name: String, port: Int)] {
        (ports ?? [:]).sorted { $0.key < $1.key }.map { (name: $0.key, port: $0.value) }
    }

    public var isAgentOwned: Bool { owner?.hasPrefix("agent:") == true }
}

/// A port reservation (`domain.Lease`).
public struct Lease: Codable, Hashable, Sendable {
    public var port: Int
    public var project: String
    public var service: String
    public var portName: String
    public var createdAt: Date?
}

/// Foreign process found on a port (`domain.PortHolder`).
public struct PortHolder: Codable, Hashable, Sendable {
    public var pid: Int
    public var command: String
    public var cwd: String?
}

/// Automatic port reassignment (`domain.PortRemap`).
public struct PortRemap: Codable, Hashable, Sendable {
    public var name: String
    public var from: Int
    public var to: Int
    public var env: String?
    public var holder: PortHolder?
    public var reason: String?
}
