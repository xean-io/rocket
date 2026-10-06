import { Circle, CircleDashed, CircleX, TriangleAlert, ZapOff } from "lucide-react";
import type { Glyph, StatusStyle, Tone } from "@/lib/status";
import { cn } from "@/lib/utils";

export const toneText: Record<Tone, string> = {
  ok: "text-status-ok",
  warn: "text-status-warn",
  bad: "text-status-bad",
  muted: "text-status-muted",
};

const icons: Record<Glyph, typeof Circle> = {
  dot: Circle,
  ring: Circle,
  dotted: CircleDashed,
  alert: TriangleAlert,
  x: CircleX,
  bolt: ZapOff,
};

/** Semantic state glyph; pulses while transitioning (unless reduced motion). */
export function StatusGlyph({ style, className }: { style: StatusStyle; className?: string }) {
  const Icon = icons[style.glyph];
  return (
    <Icon
      role="img"
      aria-label={style.label}
      className={cn(
        "size-3.5 shrink-0",
        toneText[style.tone],
        style.glyph === "dot" && "fill-current",
        style.pulses && "status-pulse",
        className,
      )}
    />
  );
}

/** A small filled dot for dense lists (sidebar rows). */
export function StatusDot({ tone, pulses, label }: { tone: Tone; pulses?: boolean; label: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className={cn(
        "size-2 shrink-0 rounded-full",
        tone === "muted" ? "border border-status-muted" : "bg-current",
        toneText[tone],
        pulses && "status-pulse",
      )}
    />
  );
}
