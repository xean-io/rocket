import RocketKit
import SwiftUI

/// Toolbar status pill: daemon connection and live-stream state (glass control layer).
struct ConnectionBadge: View {
    let state: ConnectionState
    let live: Bool

    private var style: (symbol: String, color: Color, text: String) {
        switch state {
        case .connected: live ? ("dot.radiowaves.left.and.right", .green, "Live")
                              : ("dot.radiowaves.left.and.right", .orange, "Reconnecting")
        case .connecting: ("circle.dotted", .orange, "Connecting")
        case .failed: ("exclamationmark.triangle.fill", .red, "Offline")
        case .idle: ("circle", .secondary, "Idle")
        }
    }

    var body: some View {
        Label(style.text, systemImage: style.symbol)
            .labelStyle(.titleAndIcon)
            .font(.caption.weight(.medium))
            .foregroundStyle(style.color)
            .symbolEffect(.pulse, options: .repeating, isActive: !live && state != .idle)
            .padding(.horizontal, 4)
            .accessibilityLabel("Daemon \(style.text)")
    }
}

/// Empty / error state while not connected.
struct ConnectionStateView: View {
    let state: ConnectionState
    let home: String
    let onRetry: () -> Void

    var body: some View {
        switch state {
        case .connecting, .idle:
            VStack(spacing: 14) {
                ProgressView().controlSize(.large)
                Text(message.isEmpty ? "Connecting…" : message)
                    .font(.title3)
                    .foregroundStyle(Color.xeanInk)
                MetaLabel(home)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .failed(let reason):
            ContentUnavailableView {
                Label("rocketd is not reachable", systemImage: "bolt.horizontal.circle")
            } description: {
                Text(reason)
            } actions: {
                Button("Start rocketd", action: onRetry)
                    .buttonStyle(.glassProminent)
                    .keyboardShortcut(.defaultAction)
            }
        case .connected:
            EmptyView()
        }
    }

    private var message: String {
        if case .connecting(let m) = state { return m }
        return ""
    }
}
