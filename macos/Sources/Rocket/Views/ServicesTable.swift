import RocketKit
import SwiftUI

/// Presentational services table. Content layer: opaque surface, no glass.
struct ServicesTable: View {
    let runs: [Run]
    @Binding var selection: ServiceKey?
    let onShowLogs: (ServiceKey) -> Void

    var body: some View {
        Table(runs, selection: $selection) {
            TableColumn("Service") { run in
                HStack(spacing: 8) {
                    StatusGlyph(style: StatusStyle(run.state, health: run.health))
                    Text(run.service)
                        .foregroundStyle(Color.xeanInk)
                    if let kind = run.kind { MetaLabel(kind.rawValue) }
                }
                .accessibilityElement(children: .combine)
            }
            .width(min: 160, ideal: 220)

            TableColumn("State") { run in
                VStack(alignment: .leading, spacing: 1) {
                    Text(StatusStyle(run.state, health: run.health).label)
                    if let error = run.error, !error.isEmpty {
                        Text(error).font(.caption).foregroundStyle(.red).lineLimit(1).help(error)
                    } else if let code = run.exitCode, !run.state.isActive {
                        MetaLabel("exit \(code)")
                    }
                }
            }
            .width(min: 90, ideal: 120)

            TableColumn("Ports") { run in
                Text(Format.ports(run.ports))
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle(run.ports == nil ? .secondary : Color.xeanInk)
                    .textSelection(.enabled)
            }
            .width(min: 90, ideal: 140)

            TableColumn("Owner") { run in
                HStack(spacing: 4) {
                    if run.isAgentOwned {
                        Image(systemName: "sparkles").foregroundStyle(Color.xeanViolet).accessibilityLabel("Agent")
                    }
                    MetaLabel(Format.owner(run.owner), color: run.isAgentOwned ? .xeanViolet : .xeanStone)
                }
            }
            .width(min: 80, ideal: 130)

            TableColumn("TTL") { run in
                if let expires = run.expiresAt, run.state.isActive, expires > .now {
                    Text(timerInterval: Date.now...expires, countsDown: true)
                        .monospacedDigit()
                        .foregroundStyle(.orange)
                } else {
                    Text("—").foregroundStyle(.secondary)
                }
            }
            .width(min: 60, ideal: 80)

            TableColumn("Uptime") { run in
                if let started = run.startedAt, run.state.isActive {
                    Text(started, style: .relative).monospacedDigit()
                } else {
                    Text("—").foregroundStyle(.secondary)
                }
            }
            .width(min: 70, ideal: 110)
        }
        .contextMenu(forSelectionType: ServiceKey.self) { keys in
            if let key = keys.first {
                Button("Show Logs", systemImage: "text.alignleft") { onShowLogs(key) }
            }
        } primaryAction: { keys in
            if let key = keys.first { onShowLogs(key) }
        }
        .scrollContentBackground(.hidden)
        .background(Color.xeanSurface.opacity(0.6))
        .xeanTableSelection(selection: selection, rowCount: runs.count)
        .accessibilityLabel("Services")
    }
}

/// Floating control layer: Liquid Glass action group that morphs between the
/// project-wide actions and the selected service.
struct ServiceActionBar: View {
    let selected: Run?
    let busy: Bool
    let onUp: () -> Void
    let onRestart: () -> Void
    let onStop: () -> Void
    let onLogs: () -> Void
    let onClearSelection: () -> Void

    @Namespace private var glass
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Shared height so the selection chip and the action capsule line up.
    private let barHeight: CGFloat = 40

    var body: some View {
        GlassEffectContainer(spacing: 12) {
            HStack(spacing: 12) {
                if let run = selected {
                    Button(action: onClearSelection) {
                        HStack(spacing: 8) {
                            StatusGlyph(style: StatusStyle(run.state, health: run.health))
                            Text(run.service).font(.callout.weight(.medium))
                            if !(run.ports ?? [:]).isEmpty { MetaLabel(Format.ports(run.ports)) }
                            Image(systemName: "xmark").font(.caption2).foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 14)
                        .frame(height: barHeight)
                    }
                    .buttonStyle(.plain)
                    .xeanGlass(strength: 0.28, in: Capsule())
                    .glassEffectID("chip", in: glass)
                    .help("Clear selection to act on the whole project")
                    .accessibilityLabel("Selected \(run.service). Clear selection")
                }

                HStack(spacing: 4) {
                    BarButton(title: selected == nil ? "Up All" : "Start", symbol: "play.fill", prominent: true, action: onUp)
                    BarButton(title: selected == nil ? "Restart All" : "Restart", symbol: "arrow.clockwise", action: onRestart)
                    BarButton(title: selected == nil ? "Stop All…" : "Stop", symbol: "stop.fill", action: onStop)
                    Divider().frame(height: 18)
                    BarButton(title: "Logs", symbol: "text.alignleft", action: onLogs)
                }
                .padding(.horizontal, 8)
                .frame(height: barHeight)
                .disabled(busy)
                .xeanGlass(in: Capsule())
                .glassEffectID("actions", in: glass)
            }
        }
        .animation(reduceMotion ? nil : .spring(duration: 0.35), value: selected?.id)
    }
}

private struct BarButton: View {
    let title: String
    let symbol: String
    var prominent = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.body.weight(.medium))
                .foregroundStyle(prominent ? Color.xeanViolet : Color.xeanInk)
                .frame(width: 34, height: 30)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(title)
        .accessibilityLabel(title)
    }
}
