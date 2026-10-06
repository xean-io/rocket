import SwiftUI

struct SidebarProject: Hashable, Identifiable {
    let name: String
    let path: String?
    let active: Int
    let total: Int
    var id: String { name }
}

/// Presentational sidebar: flat native rows, one icon and at most two lines.
struct SidebarView: View {
    let projects: [SidebarProject]
    let jobsAvailable: Bool
    let ownerCount: Int
    let connected: Bool
    @Binding var selection: SidebarItem?
    let onAdd: () -> Void
    let onRefresh: () -> Void
    let onRemove: (String) -> Void

    @State private var pendingRemoval: String?

    private var destinations: [SidebarItem] {
        projects.map { .project($0.name) } + [.ports, .jobs, .owners]
    }

    var body: some View {
        List {
            Section("Projects") {
                ForEach(projects) { project in
                    SidebarNavigationRow(
                        title: project.name,
                        systemImage: project.active > 0 ? "shippingbox.fill" : "shippingbox",
                        detail: project.active > 0 ? "\(project.active) of \(project.total) running" : "Idle",
                        item: .project(project.name), selection: $selection
                    )
                    .contextMenu {
                        if let path = project.path {
                            Button("Reveal in Finder", systemImage: "folder") {
                                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
                            }
                            Divider()
                            Button("Remove from Rocket…", systemImage: "minus.circle", role: .destructive) {
                                pendingRemoval = project.name
                            }
                        }
                    }
                }
                if projects.isEmpty {
                    Text("No projects yet")
                        .foregroundStyle(.secondary)
                        .selectionDisabled()
                }
                Button(action: onAdd) {
                    Label {
                        Text("Add Project…")
                    } icon: {
                        Image(systemName: "plus").foregroundStyle(Color.xeanViolet)
                    }
                }
                .buttonStyle(.plain)
                .foregroundStyle(Color.xeanViolet)
                .help("Register a directory that contains rocket.yaml (⌘O)")
                .disabled(!connected)
                .selectionDisabled()
            }
            Section("Daemon") {
                SidebarNavigationRow(title: "Ports", systemImage: "point.3.connected.trianglepath.dotted",
                                     item: .ports, selection: $selection)
                SidebarNavigationRow(title: "Jobs", systemImage: "list.bullet.rectangle",
                                     item: .jobs, selection: $selection)
                    .opacity(jobsAvailable ? 1 : 0.5)
                SidebarNavigationRow(title: "Owners", systemImage: "person.2",
                                     item: .owners, selection: $selection)
                    .badge(ownerCount)
            }
        }
        .listStyle(.sidebar)
        .onMoveCommand { direction in
            switch direction {
            case .up: moveSelection(by: -1)
            case .down: moveSelection(by: 1)
            default: break
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                Divider()
                HStack {
                    Button(action: onRefresh) {
                        Label {
                            Text("Refresh")
                        } icon: {
                            Image(systemName: "arrow.clockwise").foregroundStyle(Color.xeanViolet)
                        }
                    }
                    .disabled(!connected)
                    .help("Refresh projects and daemon state (⇧⌘R)")
                    Spacer(minLength: 8)
                    SettingsLink {
                        Label {
                            Text("Settings")
                        } icon: {
                            Image(systemName: "gearshape").foregroundStyle(Color.xeanViolet)
                        }
                    }
                    .help("Open Rocket settings (⌘,)")
                }
                .font(.callout)
                .buttonStyle(.borderless)
                .foregroundStyle(Color.xeanSecondaryInk)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
            }
        }
        .confirmationDialog("Remove \(pendingRemoval ?? "") from Rocket?",
                            isPresented: Binding(get: { pendingRemoval != nil }, set: { if !$0 { pendingRemoval = nil } }),
                            titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                if let name = pendingRemoval { onRemove(name) }
                pendingRemoval = nil
            }
        } message: {
            Text("Only the registration is removed; files stay untouched. A running project must be stopped first.")
        }
    }

    private func moveSelection(by offset: Int) {
        let current = selection.flatMap { destinations.firstIndex(of: $0) }
        let next = current.map { min(max($0 + offset, 0), destinations.count - 1) }
            ?? (offset > 0 ? 0 : destinations.count - 1)
        selection = destinations[next]
    }
}

/// Explicit selection keeps the brand color independent of macOS's accent.
private struct SidebarNavigationRow: View {
    let title: String
    let systemImage: String
    var detail: String? = nil
    let item: SidebarItem
    @Binding var selection: SidebarItem?

    private var selected: Bool { selection == item }

    var body: some View {
        Button {
            selection = item
        } label: {
            Label {
                VStack(alignment: .leading, spacing: 1) {
                    Text(title)
                        .fontWeight(selected ? .semibold : .regular)
                        .foregroundStyle(Color.xeanInk)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let detail {
                        Text(detail)
                            .font(.caption)
                            .foregroundStyle(Color.xeanSecondaryInk)
                            .lineLimit(1)
                    }
                }
            } icon: {
                Image(systemName: systemImage)
                    .foregroundStyle(selected ? Color.xeanInk : Color.xeanViolet)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(selected ? Color.xeanViolet.opacity(0.24) : .clear,
                        in: RoundedRectangle(cornerRadius: 8))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
        .selectionDisabled()
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityValue(selected ? "Selected" : "")
        .help(detail.map { "\(title): \($0)" } ?? title)
    }
}
