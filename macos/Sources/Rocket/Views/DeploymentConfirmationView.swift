import SwiftUI

/// Confirmation is explicit even when the daemon's deployment does not require it.
struct DeploymentConfirmationView: View {
    let project: String
    let env: String
    let isRerun: Bool
    let onCancel: () -> Void
    let onConfirm: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Label("Deploy to \(env)?", systemImage: "paperplane")
                .font(.title2)
            Text(isRerun ? "Run \(project)'s deployment to \(env) again."
                         : "Run \(project)'s configured deployment to \(env).")
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", action: onCancel)
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.glass)
                Button("Deploy", action: onConfirm)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.glassProminent)
            }
        }
        .padding(24)
        .frame(width: 420)
        .tint(.xeanViolet)
    }
}
