import RocketKit
import SwiftUI

/// Inspector for the selected service: facts, then the live log tail.
struct ServiceInspector: View {
    let run: Run?
    let logs: [String]
    let onReload: () -> Void

    var body: some View {
        if let run {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 8) {
                        StatusGlyph(style: StatusStyle(run.state, health: run.health))
                            .font(.title3)
                        Text(run.service)
                            .font(.title2)
                            .foregroundStyle(Color.xeanInk)
                    }
                    Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 14, verticalSpacing: 7) {
                        fact("State", StatusStyle(run.state, health: run.health).label)
                        fact("Kind", run.kind?.rawValue)
                        fact("Env", run.env)
                        fact("Owner", run.owner ?? "user")
                        fact("PID", run.pid.map(String.init))
                        fact("Ports", run.ports.map { _ in Format.ports(run.ports) })
                        fact("Compose", run.composeProject)
                        fact("Exit", run.exitCode.map(String.init))
                        if let started = run.startedAt {
                            GridRow {
                                MetaLabel("Started")
                                Text(started, format: .dateTime.hour().minute().second())
                                    .font(.callout.monospacedDigit())
                            }
                        }
                        if let expires = run.expiresAt {
                            GridRow {
                                MetaLabel("Expires")
                                Text(expires, format: .dateTime.hour().minute().second())
                                    .font(.callout.monospacedDigit())
                            }
                        }
                        fact("Log", run.logPath)
                    }
                    if let error = run.error, !error.isEmpty {
                        Text(error)
                            .font(.callout)
                            .foregroundStyle(.red)
                            .textSelection(.enabled)
                    }
                }
                .padding(16)

                Hairline()
                HStack {
                    MetaLabel("Log · \(logs.count) lines")
                    Spacer()
                    Button("Reload", systemImage: "arrow.clockwise", action: onReload)
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .help("Reload the last 300 lines")
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                LogTailView(lines: logs)
            }
        } else {
            DetailEmptyState("No Service Selected", systemImage: "sidebar.trailing",
                             description: "Select a service to see its details and live log.")
        }
    }

    @ViewBuilder
    private func fact(_ key: String, _ value: String?) -> some View {
        if let value, !value.isEmpty {
            GridRow {
                MetaLabel(key)
                Text(value)
                    .font(.callout)
                    .foregroundStyle(Color.xeanInk)
                    .textSelection(.enabled)
                    .lineLimit(2)
                    .truncationMode(.middle)
            }
        }
    }
}

/// Monospaced log tail that sticks to the bottom as lines arrive. Content, not glass.
struct LogTailView: View {
    let lines: [String]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Text(line.isEmpty ? " " : line)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(line.hasPrefix("=== rocket:") ? Color.xeanViolet : Color.xeanInk.opacity(0.88))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .textSelection(.enabled)
            .padding(12)
        }
        .defaultScrollAnchor(.bottom)
        .background(Color.xeanSurface)
        .overlay {
            if lines.isEmpty {
                Text("No output yet").foregroundStyle(.secondary)
            }
        }
        .accessibilityLabel("Log output, \(lines.count) lines")
    }
}
