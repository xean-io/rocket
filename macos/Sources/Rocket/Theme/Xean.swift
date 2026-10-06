import AppKit
import SwiftUI

// XEAN palette (source: xean/src/styles/tokens.css, brand/README.md).
// Violet is the only accent; status colours stay system semantic.
extension Color {
    static let xeanViolet = Color(hex: 0x9F63B9)
    static let xeanBone = Color(hex: 0xF3EFE6)
    static let xeanLacquer = Color(hex: 0x0B0B0B)
    static let xeanSurfaceDark = Color(hex: 0x141414)
    static let xeanBorderDark = Color(hex: 0x2C2C2C)
    static let xeanGlow = Color(hex: 0x35293B)
    static let xeanStone = Color(hex: 0x9A958C)

    /// Window ground: lacquer in dark (signature), bone in light.
    static let xeanGround = Color(light: 0xF3EFE6, dark: 0x0B0B0B)
    /// Content surface behind tables and logs.
    static let xeanSurface = Color(light: 0xFAF8F3, dark: 0x141414)
    /// Hairline separators.
    static let xeanHairline = Color(light: 0xDAD5CB, dark: 0x2C2C2C)
    /// Primary text: bone on dark, lacquer on light.
    static let xeanInk = Color(light: 0x0B0B0B, dark: 0xF3EFE6)
    /// Secondary text with readable contrast on both content grounds.
    static let xeanSecondaryInk = Color(light: 0x68645E, dark: 0x9A958C)
    /// Low radial glow behind glass controls.
    static let xeanGlowAdaptive = Color(light: 0xE9DFEE, dark: 0x35293B)

    init(hex: UInt32, opacity: Double = 1) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xFF) / 255,
                  green: Double((hex >> 8) & 0xFF) / 255,
                  blue: Double(hex & 0xFF) / 255,
                  opacity: opacity)
    }

    init(light: UInt32, dark: UInt32) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua, .vibrantDark, .vibrantLight])
                .map { $0 == .darkAqua || $0 == .vibrantDark } ?? false
            let hex = isDark ? dark : light
            return NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                           green: CGFloat((hex >> 8) & 0xFF) / 255,
                           blue: CGFloat(hex & 0xFF) / 255,
                           alpha: 1)
        })
    }
}

/// Lacquer ground with a low violet glow, the XEAN "horizon" backdrop.
struct XeanBackdrop: View {
    var body: some View {
        ZStack {
            Color.xeanGround
            RadialGradient(colors: [Color.xeanGlowAdaptive.opacity(0.9), .clear],
                           center: .init(x: 0.5, y: 1.05), startRadius: 0, endRadius: 520)
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}

/// Quiet monospaced, uppercase, tracked metadata label (ports, pids, owners).
struct MetaLabel: View {
    let text: String
    var color: Color = .xeanSecondaryInk

    init(_ text: String, color: Color = .xeanSecondaryInk) {
        self.text = text
        self.color = color
    }

    var body: some View {
        Text(text.uppercased())
            .font(.system(.caption2, design: .monospaced))
            .tracking(1.2)
            .foregroundStyle(color)
            .lineLimit(1)
            .truncationMode(.middle)
    }
}

/// A 0.5pt hairline separator.
struct Hairline: View {
    var body: some View {
        Rectangle().fill(Color.xeanHairline).frame(height: 0.5).accessibilityHidden(true)
    }
}

extension View {
    /// Liquid Glass for the control layer, tinted with XEAN violet. Falls back
    /// to an opaque bar material when Reduce Transparency is on.
    func xeanGlass<S: Shape>(tint: Color = .xeanViolet, strength: Double = 0.18, in shape: S,
                             interactive: Bool = true) -> some View {
        modifier(XeanGlassModifier(tint: tint, strength: strength, shape: shape, interactive: interactive))
    }
}

private struct XeanGlassModifier<S: Shape>: ViewModifier {
    let tint: Color
    let strength: Double
    let shape: S
    let interactive: Bool
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    func body(content: Content) -> some View {
        if reduceTransparency {
            content
                .background(Color.xeanSurface, in: shape)
                .overlay(shape.stroke(Color.xeanHairline, lineWidth: 0.5))
        } else {
            content.glassEffect(interactive ? Glass.regular.tint(tint.opacity(strength)).interactive()
                                            : Glass.regular.tint(tint.opacity(strength)),
                                in: shape)
        }
    }
}
