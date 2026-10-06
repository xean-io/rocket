import AppKit
import SwiftUI

extension View {
    /// Keep native table interaction while pinning selection to the brand accent.
    func xeanTableSelection<ID: Hashable>(selection: ID?, rowCount: Int) -> some View {
        background(TableSelectionBridge(selection: selection, rowCount: rowCount))
    }
}

/// SwiftUI Table follows the system accent even with an explicit tint on macOS.
/// This bridge only draws selection; SwiftUI still owns data, selection and delegates.
private struct TableSelectionBridge<ID: Hashable>: NSViewRepresentable {
    let selection: ID?
    let rowCount: Int
    @Environment(\.colorSchemeContrast) private var contrast

    func makeNSView(context: Context) -> TableSelectionAnchor { TableSelectionAnchor() }

    func updateNSView(_ view: TableSelectionAnchor, context: Context) {
        view.color = NSColor(Color.xeanViolet.opacity(contrast == .increased ? 0.42 : 0.24))
        view.scheduleRefresh()
    }

    static func dismantleNSView(_ view: TableSelectionAnchor, coordinator: ()) { view.detach() }
}

private final class TableSelectionAnchor: NSView {
    var color = NSColor(Color.xeanViolet.opacity(0.24))
    private weak var table: NSTableView?
    private var originalStyle: NSTableView.SelectionHighlightStyle = .regular
    private var observations: [NSObjectProtocol] = []
    private var refreshPending = false

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { detach() } else { scheduleRefresh() }
    }

    override func layout() {
        super.layout()
        scheduleRefresh()
    }

    func scheduleRefresh() {
        guard !refreshPending else { return }
        refreshPending = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.refreshPending = false
            guard self.window != nil else { return }
            self.refresh()
        }
    }

    private func refresh() {
        if table == nil {
            var ancestor = superview
            while let view = ancestor {
                if let candidate = findTable(in: view) {
                    table = candidate
                    originalStyle = candidate.selectionHighlightStyle
                    observe(NSTableView.selectionDidChangeNotification, object: candidate)
                    if let clip = candidate.enclosingScrollView?.contentView {
                        observe(NSView.boundsDidChangeNotification, object: clip)
                    }
                    break
                }
                ancestor = view.superview
            }
        }
        guard let table else { return }
        table.selectionHighlightStyle = .none
        table.enumerateAvailableRowViews { row, index in
            let fill: SelectionFill
            if let existing = row.subviews.first(where: { $0 is SelectionFill }) as? SelectionFill {
                fill = existing
            } else {
                fill = SelectionFill(frame: row.bounds)
                fill.autoresizingMask = [.width, .height]
                fill.setAccessibilityElement(false)
                row.addSubview(fill, positioned: .below, relativeTo: nil)
            }
            fill.color = color
            fill.isHidden = !table.selectedRowIndexes.contains(index)
            fill.needsDisplay = true
        }
    }

    private func findTable(in view: NSView) -> NSTableView? {
        // The sidebar is a one-column source list, not a data table.
        if let table = view as? NSTableView, table.numberOfColumns > 1 { return table }
        return view.subviews.lazy.compactMap { self.findTable(in: $0) }.first
    }

    private func observe(_ name: Notification.Name, object: AnyObject) {
        observations.append(NotificationCenter.default.addObserver(forName: name, object: object, queue: .main) {
            [weak self] _ in
            MainActor.assumeIsolated { self?.scheduleRefresh() }
        })
    }

    func detach() {
        observations.forEach(NotificationCenter.default.removeObserver)
        observations.removeAll()
        table?.enumerateAvailableRowViews { row, _ in
            row.subviews.filter { $0 is SelectionFill }.forEach { $0.removeFromSuperview() }
        }
        table?.selectionHighlightStyle = originalStyle
        table = nil
    }
}

private final class SelectionFill: NSView {
    var color = NSColor.clear

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draw(_ dirtyRect: NSRect) {
        color.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 10, dy: 0), xRadius: 8, yRadius: 8).fill()
    }
}
