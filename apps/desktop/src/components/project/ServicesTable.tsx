import { Play, RefreshCw, Sparkles, Square, TextAlignStart } from "lucide-react";
import { useRef } from "react";
import { MetaLabel } from "@/components/MetaLabel";
import { StatusGlyph } from "@/components/StatusGlyph";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useNow } from "@/hooks/useNow";
import type { Run } from "@/lib/bindings";
import { formatCountdown, formatDuration, formatOwner, parseTime, portEntries } from "@/lib/format";
import { isActive, isAgentOwned, statusStyle } from "@/lib/status";
import { cn } from "@/lib/utils";

export interface RowActions {
  onUp: (service: string) => void;
  onRestart: (service: string) => void;
  onStop: (service: string) => void;
  onShowLogs: (service: string) => void;
}

function Ports({ ports }: { ports: Run["ports"] }) {
  const entries = portEntries(ports);
  if (entries.length === 0) return <span className="text-secondary-ink">—</span>;
  return (
    <span className="selectable flex flex-wrap gap-x-3 font-mono text-[0.82rem]">
      {entries.map(([name, port]) => (
        <span key={name}>
          <span className="text-secondary-ink">{name} </span>
          {port}
        </span>
      ))}
    </span>
  );
}

/** Ticking TTL / uptime cell; subscribes to the shared clock only when shown. */
function Ticking({ run, kind }: { run: Run; kind: "ttl" | "uptime" }) {
  const now = useNow();
  if (!isActive(run.state)) return <span className="text-secondary-ink">—</span>;
  if (kind === "ttl") {
    const expires = parseTime(run.expires_at);
    if (expires === undefined || expires <= now) return <span className="text-secondary-ink">—</span>;
    return <span className="font-mono text-[0.82rem] text-status-warn tabular-nums">{formatCountdown(expires - now)}</span>;
  }
  const started = parseTime(run.started_at);
  if (started === undefined) return <span className="text-secondary-ink">—</span>;
  return <span className="font-mono text-[0.82rem] tabular-nums">{formatDuration(now - started)}</span>;
}

function RowButtons({ run, busy, actions }: { run: Run; busy: boolean; actions: RowActions }) {
  const active = isActive(run.state);
  const stop = (e: React.MouseEvent) => e.stopPropagation();
  return (
    <div
      className="flex justify-end gap-0.5 opacity-0 transition-opacity group-focus-within/row:opacity-100 group-hover/row:opacity-100 group-data-[selected=true]/row:opacity-100"
      onClick={stop}
      onDoubleClick={stop}
    >
      {active ? (
        <>
          <Button variant="ghost" size="icon-xs" disabled={busy} aria-label={`Restart ${run.service}`} title="Restart" onClick={() => actions.onRestart(run.service)}>
            <RefreshCw />
          </Button>
          <Button variant="ghost" size="icon-xs" disabled={busy} aria-label={`Stop ${run.service}`} title="Stop" onClick={() => actions.onStop(run.service)}>
            <Square />
          </Button>
        </>
      ) : (
        <Button variant="ghost" size="icon-xs" disabled={busy} aria-label={`Start ${run.service}`} title="Start" onClick={() => actions.onUp(run.service)}>
          <Play className="text-violet" />
        </Button>
      )}
    </div>
  );
}

function ServiceRow({
  run,
  selected,
  busy,
  actions,
  onSelect,
}: {
  run: Run;
  selected: boolean;
  busy: boolean;
  actions: RowActions;
  onSelect: (service: string) => void;
}) {
  const style = statusStyle(run.state, run.health);
  const agent = isAgentOwned(run.owner);
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <TableRow
            data-selected={selected}
            aria-selected={selected}
            data-service={run.service}
            onClick={() => onSelect(run.service)}
            onDoubleClick={() => actions.onShowLogs(run.service)}
            className={cn(
              "group/row h-9 cursor-default border-b-[length:var(--hairline-width)]",
              "hover:bg-primary/8 data-[selected=true]:bg-primary/20 data-[selected=true]:shadow-[inset_2px_0_0_var(--primary)]",
              "data-[selected=true]:hover:bg-primary/24",
            )}
          />
        }
      >
        <TableCell className="py-1">
          <div className="flex items-center gap-2">
            <StatusGlyph style={style} />
            <span className="truncate font-medium text-ink">{run.service}</span>
            <MetaLabel>{run.kind}</MetaLabel>
          </div>
        </TableCell>
        <TableCell className="py-1">
          <div className="flex flex-col leading-tight">
            <span>{style.label}</span>
            {run.error ? (
              <span className="max-w-56 truncate text-xs text-status-bad" title={run.error}>
                {run.error}
              </span>
            ) : run.exit_code != null && !isActive(run.state) ? (
              <MetaLabel>exit {run.exit_code}</MetaLabel>
            ) : null}
          </div>
        </TableCell>
        <TableCell className="py-1">
          <Ports ports={run.ports} />
        </TableCell>
        <TableCell className="py-1 font-mono text-[0.82rem]">
          {run.pid ? <span className="selectable">{run.pid}</span> : <span className="text-secondary-ink">—</span>}
        </TableCell>
        <TableCell className="py-1">
          <span className="flex items-center gap-1">
            {agent && <Sparkles aria-label="Agent" className="size-3 text-violet" />}
            <MetaLabel className={agent ? "text-violet" : "text-stone"}>{formatOwner(run.owner)}</MetaLabel>
          </span>
        </TableCell>
        <TableCell className="py-1">
          <Ticking run={run} kind="ttl" />
        </TableCell>
        <TableCell className="py-1">
          <Ticking run={run} kind="uptime" />
        </TableCell>
        <TableCell className="w-20 py-1 pr-3">
          <RowButtons run={run} busy={busy} actions={actions} />
        </TableCell>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-44">
        <ContextMenuItem onClick={() => actions.onShowLogs(run.service)}>
          <TextAlignStart /> Show logs
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem disabled={busy} onClick={() => actions.onUp(run.service)}>
          <Play /> Start
        </ContextMenuItem>
        <ContextMenuItem disabled={busy} onClick={() => actions.onRestart(run.service)}>
          <RefreshCw /> Restart
        </ContextMenuItem>
        <ContextMenuItem disabled={busy || !isActive(run.state)} onClick={() => actions.onStop(run.service)}>
          <Square /> Stop
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

const heads = ["Service", "State", "Ports", "PID", "Owner", "TTL", "Uptime"];

/** Services table. Arrow keys move the selection, Enter opens logs, Esc clears. */
export function ServicesTable({
  runs,
  loading,
  selected,
  busy,
  actions,
  onSelect,
}: {
  runs: readonly Run[];
  loading: boolean;
  selected: string | null;
  busy: boolean;
  actions: RowActions;
  onSelect: (service: string | null) => void;
}) {
  const root = useRef<HTMLDivElement>(null);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const i = runs.findIndex((r) => r.service === selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.min(Math.max(i + (e.key === "ArrowDown" ? 1 : -1), 0), runs.length - 1);
      const run = runs[i === -1 ? (e.key === "ArrowDown" ? 0 : runs.length - 1) : next];
      if (run) {
        onSelect(run.service);
        root.current?.querySelector(`[data-service="${CSS.escape(run.service)}"]`)?.scrollIntoView({ block: "nearest" });
      }
    } else if (e.key === "Enter" && selected) {
      actions.onShowLogs(selected);
    } else if (e.key === "Escape") {
      onSelect(null);
    }
  };

  return (
    <div
      ref={root}
      tabIndex={0}
      role="group"
      aria-label="Services"
      onKeyDown={onKeyDown}
      className="min-h-0 flex-1 overflow-auto bg-surface/60 outline-none focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
    >
      <Table className="min-w-[44rem]">
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            {heads.map((h) => (
              <TableHead key={h} className="meta h-8 font-normal">
                {h}
              </TableHead>
            ))}
            <TableHead className="w-20" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {loading
            ? Array.from({ length: 4 }, (_, i) => (
                <TableRow key={i} className="h-9 hover:bg-transparent">
                  <TableCell colSpan={8}>
                    <Skeleton className="h-4 w-full max-w-[30rem]" />
                  </TableCell>
                </TableRow>
              ))
            : runs.map((run) => (
                <ServiceRow
                  key={run.service}
                  run={run}
                  selected={run.service === selected}
                  busy={busy}
                  actions={actions}
                  onSelect={onSelect}
                />
              ))}
        </TableBody>
      </Table>
    </div>
  );
}
