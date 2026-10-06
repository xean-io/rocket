import { ArrowLeftRight, CircleDashed, Circle, TriangleAlert } from "lucide-react";
import type { Conflict, Run } from "@/lib/bindings";
import { statusCounts } from "@/lib/status";
import { cn } from "@/lib/utils";

function Capsule({
  count,
  label,
  tone,
  icon: Icon,
  pulses,
  fill,
  title,
}: {
  count: number;
  label: string;
  tone: string;
  icon: typeof Circle;
  pulses?: boolean;
  fill?: boolean;
  title?: string;
}) {
  return (
    <span
      title={title ?? `${count} ${label}`}
      className="flex shrink-0 items-center gap-1.5 rounded-full border border-border bg-surface/80 px-2.5 py-0.5 text-sm"
    >
      <Icon aria-hidden className={cn("size-3.5", tone, fill && "fill-current", pulses && "status-pulse")} />
      <span className="tabular-nums">{count}</span>
      {/* Labels give way to bare counts when the header gets narrow. */}
      <span className="@max-xl:sr-only text-secondary-ink">{label}</span>
    </span>
  );
}

/** Running / changing / down counts (plus port issues and a working marker). */
export function StatusCapsules({
  runs,
  conflicts = [],
  busy = false,
}: {
  runs: readonly Run[];
  conflicts?: readonly Conflict[];
  busy?: boolean;
}) {
  const { running, changing, down } = statusCounts(runs);
  return (
    <div className="flex items-center gap-2" aria-label="Service status">
      {busy && (
        <span className="flex items-center gap-1.5 rounded-full border border-violet/50 bg-surface/80 px-2.5 py-0.5 text-sm">
          <CircleDashed aria-hidden className="size-3.5 text-violet status-pulse" />
          Working
        </span>
      )}
      <Capsule count={running} label="running" tone={running > 0 ? "text-status-ok" : "text-status-muted"} icon={Circle} fill />
      {changing > 0 && <Capsule count={changing} label="changing" tone="text-status-warn" icon={CircleDashed} pulses />}
      {down > 0 && <Capsule count={down} label="down" tone="text-status-bad" icon={TriangleAlert} />}
      {conflicts.length > 0 && (
        <Capsule
          count={conflicts.length}
          label={conflicts.length === 1 ? "port issue" : "port issues"}
          tone="text-status-warn"
          icon={ArrowLeftRight}
          title={conflicts.map((c) => c.detail).join("\n")}
        />
      )}
    </div>
  );
}
