import RocketKit
import SwiftUI

/// Global port map. Remapped leases and blocked ports are highlighted.
struct PortsView: View {
    let ports: [PortInfo]
    let runs: [ServiceKey: Run]
    let conflicts: [Conflict]
    let onShowService: (ServiceKey) -> Void

    @State private var selection: PortInfo.ID?

    private func remap(for p: PortInfo) -> Conflict? {
        conflicts.first { $0.isRemap && $0.project == p.project && $0.service == p.service && $0.portName == p.portName }
    }

    private var blocked: [Conflict] { conflicts.filter { !$0.isRemap } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader(title: "Ports", detail: "\(ports.count) leased · \(conflicts.filter(\.isRemap).count) remapped · \(blocked.count) blocked")
            Hairline()
            if !blocked.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(blocked) { c in
                        Label {
                            Text("\(c.project)/\(c.service) — \(c.detail ?? "port \(c.port) busy")")
                        } icon: {
                            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                        }
                        .font(.callout)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 10)
                Hairline()
            }
            if ports.isEmpty {
                DetailEmptyState("No Leased Ports", systemImage: "point.3.connected.trianglepath.dotted",
                                 description: "Ports appear here while services hold them.")
            } else {
                Table(ports, selection: $selection) {
                    TableColumn("Port") { p in
                        HStack(spacing: 6) {
                            Text(String(p.port))
                                .font(.system(.body, design: .monospaced))
                                .foregroundStyle(remap(for: p) != nil ? .orange : Color.xeanInk)
                            if let r = remap(for: p) {
                                MetaLabel("from \(r.default)", color: .orange)
                                    .help(r.detail ?? "")
                            }
                        }
                        .accessibilityElement(children: .combine)
                    }
                    .width(min: 90, ideal: 150)
                    TableColumn("Service") { p in Text("\(p.project)/\(p.service)") }
                    TableColumn("Name") { p in MetaLabel(p.portName) }
                        .width(min: 50, ideal: 80)
                    TableColumn("State") { p in
                        let state = runs[ServiceKey(project: p.project, service: p.service)]?.state ?? p.state ?? .stopped
                        HStack(spacing: 6) {
                            StatusGlyph(style: StatusStyle(state))
                            Text(StatusStyle(state).label)
                        }
                    }
                    .width(min: 80, ideal: 110)
                    TableColumn("Owner") { p in MetaLabel(Format.owner(p.owner)) }
                    TableColumn("PID") { p in
                        Text(p.pid.map(String.init) ?? "—").font(.system(.body, design: .monospaced)).foregroundStyle(.secondary)
                    }
                    .width(min: 50, ideal: 70)
                }
                .contextMenu(forSelectionType: PortInfo.ID.self) { ids in
                    if let p = ports.first(where: { ids.contains($0.id) }) {
                        Button("Show Service", systemImage: "arrow.forward.circle") {
                            onShowService(ServiceKey(project: p.project, service: p.service))
                        }
                        Button("Copy URL", systemImage: "doc.on.doc") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString("http://127.0.0.1:\(p.port)", forType: .string)
                        }
                    }
                } primaryAction: { ids in
                    if let p = ports.first(where: { ids.contains($0.id) }) {
                        onShowService(ServiceKey(project: p.project, service: p.service))
                    }
                }
                .scrollContentBackground(.hidden)
                .background(Color.xeanSurface.opacity(0.6))
                .xeanTableSelection(selection: selection, rowCount: ports.count)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .navigationTitle(VerificationRuntime.title(for: "Ports"))
    }
}

/// Section title with a quiet metadata line.
struct SectionHeader: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.system(.largeTitle, weight: .regular))
                .foregroundStyle(Color.xeanInk)
            MetaLabel(detail)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 24)
        .padding(.top, 20)
        .padding(.bottom, 14)
    }
}
