import RocketKit
import SwiftUI

/// Menu bar extra window: running count, per-project quick list, stop all.
struct MenuBarContent: View {
    @Bindable var controller: RocketController
    @Bindable var navigation: AppNavigation
    @Environment(\.openWindow) private var openWindow
    @State private var confirmingStopAll = false

    private var store: RocketStore { controller.store }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(store.runningCount)")
                        .font(.system(size: 30, weight: .light))
                        .monospacedDigit()
                        .contentTransition(.numericText())
                    MetaLabel(controller.connection.isConnected ? "services running" : "rocketd offline")
                }
                Spacer()
                ConnectionBadge(state: controller.connection, live: controller.streamLive)
            }
            .padding(14)

            Hairline()

            if controller.connection.isConnected {
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(store.projectNames, id: \.self) { name in
                            MenuProjectRow(name: shortTitle(name),
                                           active: store.activeCount(in: name),
                                           total: store.runs(in: name).count,
                                           onOpen: { open(.project(name)) },
                                           onUp: { Task { await controller.up(project: name) } },
                                           onDown: { Task { await controller.down(project: name) } })
                        }
                        if store.projectNames.isEmpty {
                            Text("No projects").foregroundStyle(.secondary).padding(8)
                        }
                    }
                    .padding(6)
                }
                .frame(maxHeight: 280)
            } else {
                Button("Start rocketd") { Task { await controller.connect() } }
                    .buttonStyle(.glassProminent)
                    .padding(14)
            }

            Hairline()

            VStack(alignment: .leading, spacing: 8) {
                if confirmingStopAll {
                    HStack {
                        Text("Stop every service?").font(.callout)
                        Spacer()
                        Button("Cancel") { confirmingStopAll = false }
                            .buttonStyle(.glass)
                        Button("Stop All", role: .destructive) {
                            confirmingStopAll = false
                            Task { await controller.stopAll() }
                        }
                        .buttonStyle(.glassProminent)
                    }
                } else {
                    Button("Stop All…", systemImage: "stop.circle") { confirmingStopAll = true }
                        .buttonStyle(.glass)
                        .disabled(store.runningCount == 0 || !controller.connection.isConnected)
                }
                HStack {
                    Button("Open Rocket") { open(nil) }
                        .keyboardShortcut("o")
                    SettingsLink { Text("Settings…") }
                    Spacer()
                    Button("Quit") { NSApplication.shared.terminate(nil) }
                        .keyboardShortcut("q")
                }
                .buttonStyle(.borderless)
                .font(.callout)
            }
            .padding(14)
        }
        .frame(width: 320)
        .task {
            if controller.connection == .idle { await controller.connect() }
        }
    }

    private func open(_ item: SidebarItem?) {
        if let item { navigation.selection = item }
        openWindow(id: "main")
        NSApp.activate()
    }

    /// Menu bar labels stay short (HIG: at most ~30 characters).
    private func shortTitle(_ s: String) -> String {
        s.count <= 30 ? s : String(s.prefix(27)) + "…"
    }
}

private struct MenuProjectRow: View {
    let name: String
    let active: Int
    let total: Int
    let onOpen: () -> Void
    let onUp: () -> Void
    let onDown: () -> Void
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onOpen) {
                HStack(spacing: 8) {
                    Image(systemName: active > 0 ? "circle.fill" : "circle")
                        .font(.caption2)
                        .foregroundStyle(active > 0 ? Color.green : .secondary)
                    Text(name).lineLimit(1)
                    Spacer()
                    MetaLabel("\(active)/\(total)")
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(name), \(active) of \(total) running. Open")

            if active > 0 {
                Button("Stop \(name)", systemImage: "stop.fill", action: onDown)
            } else {
                Button("Start \(name)", systemImage: "play.fill", action: onUp)
            }
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(hovering ? Color.primary.opacity(0.06) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .onHover { hovering = $0 }
    }
}
