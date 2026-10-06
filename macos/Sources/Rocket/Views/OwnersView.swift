import RocketKit
import SwiftUI

/// Container: what each owner (user or agent) left running.
struct OwnersContainer: View {
    @Bindable var controller: RocketController
    @Bindable var navigation: AppNavigation

    var body: some View {
        OwnersView(groups: controller.store.ownerGroups,
                   runningJobs: controller.store.jobs.filter { $0.status == .running },
                   busy: controller.isBusy("down:*"),
                   onStopOwner: { owner in Task { await controller.stopAll(owner: owner) } },
                   onShowService: { navigation.showLogs(for: $0) })
            .navigationTitle(VerificationRuntime.title(for: "Owners"))
    }
}

/// Presentational owner groups with a confirmed "Stop all for owner".
struct OwnersView: View {
    let groups: [OwnerGroup]
    let runningJobs: [Job]
    let busy: Bool
    let onStopOwner: (String) -> Void
    let onShowService: (ServiceKey) -> Void

    @State private var pendingOwner: String?

    var body: some View {
        VStack(spacing: 0) {
            SectionHeader(title: "Owners",
                          detail: "\(groups.count) owners · \(groups.reduce(0) { $0 + $1.runs.count }) active runs")
            Hairline()
            if groups.isEmpty {
                DetailEmptyState("Nothing Running", systemImage: "person.2",
                                 description: "Runs started by you or by agents (ROCKET_OWNER=agent:…) are grouped here.")
            } else {
                List {
                    ForEach(groups) { group in
                        Section {
                            ForEach(group.runs) { run in
                                Button { onShowService(run.id) } label: {
                                    HStack(spacing: 10) {
                                        StatusGlyph(style: StatusStyle(run.state, health: run.health))
                                        Text("\(run.project)/\(run.service)").foregroundStyle(Color.xeanInk)
                                        Spacer()
                                        MetaLabel(Format.ports(run.ports))
                                        if let expires = run.expiresAt, expires > .now {
                                            Text(timerInterval: Date.now...expires, countsDown: true)
                                                .monospacedDigit()
                                                .foregroundStyle(.orange)
                                                .frame(minWidth: 56, alignment: .trailing)
                                        }
                                    }
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .accessibilityHint("Shows the service")
                            }
                            let jobs = runningJobs.filter { ($0.owner ?? "user") == group.owner }
                            ForEach(jobs) { job in
                                HStack(spacing: 10) {
                                    StatusGlyph(style: StatusStyle(job.status))
                                    Text("\(job.project) · \(job.kind.rawValue) \(job.name)")
                                    Spacer()
                                    MetaLabel("job")
                                }
                            }
                        } header: {
                            HStack {
                                Label(group.owner, systemImage: group.isAgent ? "sparkles" : "person")
                                    .foregroundStyle(group.isAgent ? Color.xeanViolet : Color.xeanInk)
                                    .font(.headline)
                                Spacer()
                                Button("Stop All for Owner…", systemImage: "stop.circle") { pendingOwner = group.owner }
                                    .buttonStyle(.glass)
                                    .controlSize(.small)
                                    .disabled(busy)
                                    .accessibilityLabel("Stop all for \(group.owner)")
                            }
                            .padding(.vertical, 4)
                        }
                    }
                }
                .scrollContentBackground(.hidden)
                .background(Color.xeanSurface.opacity(0.6))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .confirmationDialog("Stop everything started by \(pendingOwner ?? "")?",
                            isPresented: Binding(get: { pendingOwner != nil }, set: { if !$0 { pendingOwner = nil } }),
                            titleVisibility: .visible) {
            Button("Stop All", role: .destructive) {
                if let owner = pendingOwner { onStopOwner(owner) }
                pendingOwner = nil
            }
        } message: {
            Text("Stops this owner's services in every project and cancels its running jobs.")
        }
    }
}
