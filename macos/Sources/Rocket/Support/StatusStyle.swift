import RocketKit
import SwiftUI

/// Visual vocabulary for run and job states: SF Symbols + system semantic colours.
struct StatusStyle {
    let symbol: String
    let color: Color
    let label: String
    let pulses: Bool

    init(_ state: RunState, health: Health? = nil) {
        switch state {
        case .running where health == .unhealthy:
            self.init("exclamationmark.circle.fill", .orange, "Unhealthy")
        case .running:
            self.init("circle.fill", .green, "Running")
        case .starting:
            self.init("circle.dotted", .orange, "Starting", pulses: true)
        case .stopping:
            self.init("circle.dotted", .secondary, "Stopping", pulses: true)
        case .stopped:
            self.init("circle", .secondary, "Stopped")
        case .exited:
            self.init("xmark.circle", .orange, "Exited")
        case .failed:
            self.init("exclamationmark.triangle.fill", .red, "Failed")
        case .dead:
            self.init("bolt.slash.fill", .red, "Dead")
        default:
            self.init("questionmark.circle", .secondary, state.rawValue.capitalized)
        }
    }

    init(_ status: JobStatus) {
        switch status {
        case .running: self.init("arrow.triangle.2.circlepath", .orange, "Running", pulses: true)
        case .succeeded: self.init("checkmark.circle.fill", .green, "Succeeded")
        case .failed: self.init("xmark.octagon.fill", .red, "Failed")
        case .canceled: self.init("slash.circle", .secondary, "Canceled")
        case .lost: self.init("questionmark.circle", .orange, "Lost")
        default: self.init("questionmark.circle", .secondary, status.rawValue.capitalized)
        }
    }

    private init(_ symbol: String, _ color: Color, _ label: String, pulses: Bool = false) {
        self.symbol = symbol
        self.color = color
        self.label = label
        self.pulses = pulses
    }
}

/// State glyph that pulses while transitioning (unless Reduce Motion is on).
struct StatusGlyph: View {
    let style: StatusStyle
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Image(systemName: style.symbol)
            .symbolRenderingMode(.hierarchical)
            .foregroundStyle(style.color)
            .symbolEffect(.pulse, options: .repeating, isActive: style.pulses && !reduceMotion)
            .accessibilityLabel(style.label)
    }
}

enum Format {
    static func ports(_ ports: [String: Int]?) -> String {
        guard let ports, !ports.isEmpty else { return "—" }
        return ports.sorted { $0.key < $1.key }.map { "\($0.key) \($0.value)" }.joined(separator: "  ")
    }

    static func duration(ms: Int?) -> String {
        guard let ms else { return "—" }
        let d = Duration.milliseconds(ms)
        return d.formatted(.units(allowed: [.hours, .minutes, .seconds], width: .narrow, maximumUnitCount: 2))
    }

    static func owner(_ owner: String?) -> String { owner ?? "user" }
}
