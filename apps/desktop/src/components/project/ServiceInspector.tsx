import { RefreshCw, TextAlignStart } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { LogTail } from "@/components/project/LogTail";
import { MetaLabel } from "@/components/MetaLabel";
import { StatusGlyph } from "@/components/StatusGlyph";
import { Button } from "@/components/ui/button";
import { useServiceLogs } from "@/hooks/useServiceLogs";
import type { Run } from "@/lib/bindings";
import { formatClock, formatOwner, portEntries } from "@/lib/format";
import { statusStyle } from "@/lib/status";

function Fact({ label, value, mono = true }: { label: string; value?: string | null; mono?: boolean }) {
  if (!value) return null;
  return (
    <>
      <MetaLabel className="pt-0.5">{label}</MetaLabel>
      <span className={`selectable line-clamp-2 break-all text-sm text-ink ${mono ? "font-mono text-[0.8rem]" : ""}`}>
        {value}
      </span>
    </>
  );
}

/** Facts for the selected service plus its live log tail. */
export function ServiceInspector({ run }: { run: Run | null }) {
  // The follow lives and dies with this component: it stops when the pane
  // closes and restarts (keyed remount in the parent) when the service changes.
  const logs = useServiceLogs(run?.project, run?.service, run !== null);

  if (!run) {
    return (
      <EmptyState
        icon={TextAlignStart}
        title="No service selected"
        description="Select a service to see its details and live log."
      />
    );
  }
  const style = statusStyle(run.state, run.health);
  const ports = portEntries(run.ports)
    .map(([n, p]) => `${n} ${p}`)
    .join("   ");
  return (
    <div className="flex h-full min-h-0 flex-col bg-surface/40">
      <div className="space-y-3 p-4">
        <div className="flex items-center gap-2">
          <StatusGlyph style={style} className="size-4" />
          <h3 className="truncate text-lg font-normal tracking-tight text-ink">{run.service}</h3>
        </div>
        <div className="grid grid-cols-[4.5rem_1fr] gap-x-3 gap-y-1.5">
          <Fact label="State" value={style.label} mono={false} />
          <Fact label="Kind" value={run.kind} />
          <Fact label="Env" value={run.env} />
          <Fact label="Owner" value={formatOwner(run.owner)} />
          <Fact label="PID" value={run.pid?.toString()} />
          <Fact label="Ports" value={ports} />
          <Fact label="Compose" value={run.compose_project} />
          <Fact label="Exit" value={run.exit_code?.toString()} />
          <Fact label="Started" value={formatClock(run.started_at)} />
          <Fact label="Expires" value={formatClock(run.expires_at)} />
          <Fact label="Log" value={run.log_path} />
        </div>
        {run.error && <p className="selectable text-sm text-status-bad">{run.error}</p>}
      </div>
      <div className="hairline-t hairline-b flex items-center gap-2 px-4 py-1.5">
        <MetaLabel className="flex-1">Log · {logs.lines.length} lines</MetaLabel>
        <Button variant="ghost" size="icon-xs" onClick={logs.reload} aria-label="Reload log" title="Reload the last 300 lines">
          <RefreshCw />
        </Button>
      </div>
      {logs.error && <p className="selectable px-4 py-2 text-xs text-status-bad">{logs.error}</p>}
      <LogTail lines={logs.lines} />
    </div>
  );
}
