import { Check, CircleDashed, CircleX, ListChecks, RefreshCw, RotateCw, Square } from "lucide-react";
import { useMemo } from "react";
import { EmptyState } from "@/components/EmptyState";
import { MetaLabel } from "@/components/MetaLabel";
import { LogTail } from "@/components/project/LogTail";
import { SectionHeader } from "@/components/SectionHeader";
import { TopBar } from "@/components/shell/TopBar";
import { StatusGlyph } from "@/components/StatusGlyph";
import { Button } from "@/components/ui/button";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "@/components/ui/context-menu";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useElementWidth } from "@/hooks/useElementWidth";
import { useJobActions } from "@/hooks/useJobActions";
import { useJobLogs } from "@/hooks/useJobLogs";
import { useNow } from "@/hooks/useNow";
import type { Job } from "@/lib/bindings";
import { formatClock, formatDuration, formatOwner, formatRelative } from "@/lib/format";
import { jobDuration, jobTitle, runningJobs, stepLabel, stepState } from "@/lib/jobs";
import { useNav } from "@/lib/nav";
import { useJobs } from "@/lib/queries";
import { jobStatusStyle } from "@/lib/status";
import { cn } from "@/lib/utils";

/** Below this table width the Step, Owner and Started columns are dropped. */
export const COMPACT_BELOW_PX = 780;

type Actions = ReturnType<typeof useJobActions>;

function Elapsed({ job }: { job: Job }) {
  const now = useNow();
  return <span className="font-mono text-[0.82rem] tabular-nums">{formatDuration(jobDuration(job, now))}</span>;
}

function Started({ job }: { job: Job }) {
  const now = useNow();
  return <span className="text-secondary-ink">{formatRelative(job.started_at, now)}</span>;
}

function JobRow({
  job,
  compact,
  selected,
  actions,
  onSelect,
}: {
  job: Job;
  compact: boolean;
  selected: boolean;
  actions: Actions;
  onSelect: (id: string) => void;
}) {
  const style = jobStatusStyle(job.status);
  const failed = (job.exit_code ?? 0) !== 0;
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <TableRow
            data-selected={selected}
            aria-selected={selected}
            data-job={job.id}
            onClick={() => onSelect(job.id)}
            onContextMenu={() => onSelect(job.id)}
            className={cn(
              "h-9 cursor-default border-b-[length:var(--hairline-width)]",
              "hover:bg-primary/8 data-[selected=true]:bg-primary/20 data-[selected=true]:shadow-[inset_2px_0_0_var(--primary)]",
            )}
          />
        }
      >
        <TableCell className="py-1">
          <div className="flex items-center gap-2">
            <StatusGlyph style={style} />
            <span className="truncate font-medium text-ink">{jobTitle(job)}</span>
            {!compact && <MetaLabel>{job.kind}</MetaLabel>}
          </div>
        </TableCell>
        <TableCell className="py-1">
          <span className="block max-w-40 truncate">{job.project}</span>
        </TableCell>
        {!compact && (
          <TableCell className="py-1 font-mono text-[0.82rem] tabular-nums">
            {job.steps.length === 0 ? "—" : `${job.step ?? 0}/${job.steps.length}`}
          </TableCell>
        )}
        <TableCell className={cn("py-1 font-mono text-[0.82rem]", failed ? "text-status-bad" : "text-secondary-ink")}>
          {job.exit_code ?? "—"}
        </TableCell>
        {!compact && (
          <TableCell className="py-1">
            <MetaLabel>{formatOwner(job.owner)}</MetaLabel>
          </TableCell>
        )}
        {!compact && (
          <TableCell className="py-1">
            <Started job={job} />
          </TableCell>
        )}
        <TableCell className="py-1">
          <Elapsed job={job} />
        </TableCell>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-44">
        {job.status === "running" ? (
          <ContextMenuItem variant="destructive" onClick={() => actions.cancel.mutate(job)}>
            <Square /> Cancel Job
          </ContextMenuItem>
        ) : (
          <ContextMenuItem onClick={() => actions.rerun.mutate(job)}>
            <RotateCw /> Run Again
          </ContextMenuItem>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}

function JobsTable({
  jobs,
  loading,
  selected,
  actions,
  onSelect,
}: {
  jobs: readonly Job[];
  loading: boolean;
  selected: string | null;
  actions: Actions;
  onSelect: (id: string | null) => void;
}) {
  const { ref, width } = useElementWidth<HTMLDivElement>();
  const compact = width !== null && width < COMPACT_BELOW_PX;
  const heads = compact
    ? ["Job", "Project", "Exit", "Duration"]
    : ["Job", "Project", "Step", "Exit", "Owner", "Started", "Duration"];

  const onKeyDown = (e: React.KeyboardEvent) => {
    const i = jobs.findIndex((j) => j.id === selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.min(Math.max(i + (e.key === "ArrowDown" ? 1 : -1), 0), jobs.length - 1);
      const job = jobs[i === -1 ? (e.key === "ArrowDown" ? 0 : jobs.length - 1) : next];
      if (job) {
        onSelect(job.id);
        ref.current?.querySelector(`[data-job="${CSS.escape(job.id)}"]`)?.scrollIntoView({ block: "nearest" });
      }
    } else if (e.key === "Escape") {
      onSelect(null);
    }
  };

  return (
    <div
      ref={ref}
      tabIndex={0}
      role="group"
      aria-label="Jobs"
      onKeyDown={onKeyDown}
      className="min-h-0 flex-1 overflow-auto bg-surface/60 outline-none focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
    >
      <Table className={compact ? "min-w-[26rem]" : "min-w-[46rem]"}>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            {heads.map((h) => (
              <TableHead key={h} className="meta h-8 font-normal">
                {h}
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {loading
            ? Array.from({ length: 4 }, (_, i) => (
                <TableRow key={i} className="h-9 hover:bg-transparent">
                  <TableCell colSpan={heads.length}>
                    <Skeleton className="h-4 w-full max-w-[30rem]" />
                  </TableCell>
                </TableRow>
              ))
            : jobs.map((job) => (
                <JobRow
                  key={job.id}
                  job={job}
                  compact={compact}
                  selected={job.id === selected}
                  actions={actions}
                  onSelect={onSelect}
                />
              ))}
        </TableBody>
      </Table>
    </div>
  );
}

function StepIcon({ state }: { state: ReturnType<typeof stepState> }) {
  if (state === "done") return <Check className="size-3.5 text-status-ok" aria-label="Done" />;
  if (state === "failed") return <CircleX className="size-3.5 text-status-bad" aria-label="Failed" />;
  if (state === "current") return <CircleDashed className="status-pulse size-3.5 text-status-warn" aria-label="Running" />;
  return <span className="size-3.5 rounded-full border border-status-muted" aria-label="Pending" />;
}

function Fact({ label, value }: { label: string; value?: string | null }) {
  if (!value) return null;
  return (
    <>
      <MetaLabel className="pt-0.5">{label}</MetaLabel>
      <span className="selectable line-clamp-2 font-mono text-[0.8rem] break-all text-ink">{value}</span>
    </>
  );
}

/** Facts, steps and the followed log of one job. The follow stops with this component. */
function JobInspector({ job }: { job: Job }) {
  const logs = useJobLogs(job.id);
  const now = useNow();
  const style = jobStatusStyle(job.status);
  const finished = job.status !== "running";
  return (
    <div className="flex h-full min-h-0 flex-col bg-surface/40">
      <div className="space-y-3 overflow-auto p-4">
        <div className="flex items-center gap-2">
          <StatusGlyph style={style} className="size-4" />
          <h3 className="truncate text-lg font-normal tracking-tight text-ink">{jobTitle(job)}</h3>
        </div>
        <div className="grid grid-cols-[4.5rem_1fr] gap-x-3 gap-y-1.5">
          <Fact label="Kind" value={job.kind} />
          <Fact label="Project" value={job.project} />
          <Fact label="Id" value={job.id} />
          <Fact label="Env" value={job.env} />
          <Fact label="Owner" value={formatOwner(job.owner)} />
          <Fact label="Started" value={formatClock(job.started_at)} />
          <Fact label="Elapsed" value={formatDuration(jobDuration(job, now))} />
          <Fact label="Log" value={job.log_path} />
        </div>
        {job.steps.length > 0 && (
          <ol aria-label="Steps" className="space-y-1">
            {job.steps.map((step, i) => (
              <li key={i} className="flex items-center gap-2">
                <StepIcon state={stepState(job, i)} />
                <span className="selectable truncate font-mono text-[0.8rem]">{stepLabel(step)}</span>
              </li>
            ))}
          </ol>
        )}
        {job.error && <p className="selectable text-sm text-status-bad">{job.error}</p>}
      </div>
      <div className="hairline-t hairline-b flex items-center gap-2 px-4 py-1.5">
        <MetaLabel className="flex-1">Log · {logs.lines.length} lines</MetaLabel>
        <Button variant="ghost" size="icon-xs" onClick={logs.reload} aria-label="Reload log" title="Replay the log from the start">
          <RefreshCw />
        </Button>
      </div>
      {logs.error && <p className="selectable px-4 py-2 text-xs text-status-bad">{logs.error}</p>}
      <LogTail lines={logs.lines} emptyText={finished ? "This job wrote no output" : "Waiting for output…"} />
      {(logs.ended || finished) && (
        <div role="status" className={cn("hairline-t flex items-center gap-2 px-4 py-1.5 text-xs", style.tone === "bad" ? "text-status-bad" : "text-secondary-ink")}>
          <StatusGlyph style={style} />
          <span>
            {finished
              ? `${style.label}${job.exit_code != null ? ` (exit ${job.exit_code})` : ""} · ${formatDuration(job.duration_ms)}`
              : "Log stream ended"}
          </span>
        </div>
      )}
    </div>
  );
}

/** Job history with Cancel / Run Again and a live log for the selected job. */
export function JobsPage() {
  const { jobs, isPending, unavailable } = useJobs();
  const actions = useJobActions();
  const selectedId = useNav((s) => s.selectedJob);
  const selectJob = useNav((s) => s.selectJob);
  const selected = useMemo(() => jobs.find((j) => j.id === selectedId) ?? null, [jobs, selectedId]);
  const running = runningJobs(jobs).length;

  const toolbar = (
    <>
      {selected &&
        (selected.status === "running" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={actions.cancel.isPending}
            onClick={() => actions.cancel.mutate(selected)}
          >
            <Square /> Cancel
          </Button>
        ) : (
          <Button size="sm" disabled={actions.rerun.isPending} onClick={() => actions.rerun.mutate(selected)}>
            <RotateCw /> Run Again
          </Button>
        ))}
    </>
  );

  if (unavailable) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <TopBar title="Jobs" />
        <EmptyState
          icon={ListChecks}
          title="Jobs not available"
          description="This rocketd does not expose the jobs API yet. Update rocket and restart the daemon."
        />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <TopBar title="Jobs">{toolbar}</TopBar>
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel id="jobs" minSize={360}>
          <div className="flex h-full min-h-0 flex-col">
            <SectionHeader title="Jobs" detail={`${running} running · ${jobs.length} recent`} />
            <div className="hairline-t flex min-h-0 flex-1 flex-col">
              {!isPending && jobs.length === 0 ? (
                <EmptyState
                  icon={ListChecks}
                  title="No jobs yet"
                  description="Pipelines, setup and deploys started with rocket run, setup or deploy appear here."
                />
              ) : (
                <JobsTable jobs={jobs} loading={isPending} selected={selected?.id ?? null} actions={actions} onSelect={selectJob} />
              )}
            </div>
          </div>
        </ResizablePanel>
        {selected && (
          <>
            <ResizableHandle />
            <ResizablePanel id="job-inspector" defaultSize={420} minSize={300} maxSize={680} groupResizeBehavior="preserve-pixel-size">
              <JobInspector key={selected.id} job={selected} />
            </ResizablePanel>
          </>
        )}
      </ResizablePanelGroup>
    </div>
  );
}
