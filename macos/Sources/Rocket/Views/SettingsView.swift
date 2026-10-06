import RocketKit
import SwiftUI

/// Settings: daemon status, ROCKET_HOME, restart daemon.
struct SettingsView: View {
    @Bindable var controller: RocketController
    @State private var confirmRestart = false

    var body: some View {
        Form {
            Section("Daemon") {
                LabeledContent("Status") {
                    ConnectionBadge(state: controller.connection, live: controller.streamLive)
                }
                if case .connected(let health) = controller.connection {
                    LabeledContent("Version") { Text(health.version).textSelection(.enabled) }
                    LabeledContent("API") { Text(health.api) }
                    LabeledContent("PID") { Text(String(health.pid)).monospacedDigit() }
                    if let started = health.startedAt {
                        LabeledContent("Started") { Text(started, format: .dateTime) }
                    }
                }
                if let http = controller.config?.http {
                    LabeledContent("Listener") { Text(http.absoluteString).font(.system(.body, design: .monospaced)) }
                }
                if case .failed(let reason) = controller.connection {
                    Text(reason).foregroundStyle(.red).font(.callout)
                }
            }

            Section("Location") {
                LabeledContent("ROCKET_HOME") {
                    Text(controller.home.path)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                }
                LabeledContent("daemon.json") {
                    Text(controller.configURL.lastPathComponent).font(.system(.body, design: .monospaced))
                }
                ForEach(controller.configWarnings, id: \.self) { warning in
                    Label(warning, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.callout)
                }
                Button("Reveal in Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([controller.home])
                }
            }

            Section {
                HStack {
                    Button("Reconnect") { Task { await controller.connect() } }
                        .buttonStyle(.glass)
                    Button("Restart Daemon…") { confirmRestart = true }
                        .buttonStyle(.glassProminent)
                    Spacer()
                    Button("Collect Garbage") { Task { await controller.gc() } }
                        .disabled(!controller.connection.isConnected)
                }
            } footer: {
                Text("Restarting rocketd keeps running services: the new daemon adopts them on start.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .confirmationDialog("Restart rocketd?", isPresented: $confirmRestart) {
            Button("Restart", role: .destructive) { Task { await controller.restartDaemon() } }
        } message: {
            Text("The app reconnects automatically.")
        }
    }
}
