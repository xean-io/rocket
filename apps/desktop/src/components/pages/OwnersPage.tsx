import { CircleStop, Sparkles, User, Users } from "lucide-react";
import { useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { EmptyState } from "@/components/EmptyState";
import { MetaLabel } from "@/components/MetaLabel";
import { SectionHeader } from "@/components/SectionHeader";
import { projectPath } from "@/components/shell/AppSidebar";
import { TopBar } from "@/components/shell/TopBar";
import { StatusGlyph } from "@/components/StatusGlyph";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { useNow } from "@/hooks/useNow";
import { useStopOwner } from "@/hooks/useStopOwner";
import type { Run } from "@/lib/bindings";
import { formatCountdown, parseTime, portEntries } from "@/lib/format";
import { jobTitle } from "@/lib/jobs";
import { useNav } from "@/lib/nav";
import { type OwnerGroup, ownerGroups } from "@/lib/owners";
import { useJobs, useRuns } from "@/lib/queries";
import { statusStyle } from "@/lib/status";
import { cn } from "@/lib/utils";

function Expiry({ run }: { run: Run }) {
  const now = useNow();
  const expires = parseTime(run.expires_at);
  if (expires === undefined || expires <= now) return null;
  return <span className="font-mono text-[0.82rem] text-status-warn tabular-nums">{formatCountdown(expires - now)}</span>;
}

function GroupSection({
  group,
  busy,
  onShow,
  onStop,
}: {
  group: OwnerGroup;
  busy: boolean;
  onShow: (run: Run) => void;
  onStop: (owner: string) => void;
}) {
  const Icon = group.isAgent ? Sparkles : User;
  return (
    <section aria-label={`Owner ${group.owner}`} className="hairline-b">
      <header className="flex items-center gap-2 px-6 pt-3 pb-1.5">
        <Icon className={cn("size-4", group.isAgent ? "text-violet" : "text-secondary-ink")} aria-hidden />
        <h3 className={cn("min-w-0 flex-1 truncate text-sm font-medium", group.isAgent ? "text-violet" : "text-ink")}>
          {group.owner}
        </h3>
        <MetaLabel className="shrink-0">
          {group.runs.length} {group.runs.length === 1 ? "service" : "services"} · {group.jobs.length}{" "}
          {group.jobs.length === 1 ? "job" : "jobs"}
        </MetaLabel>
        <Button
          variant="outline"
          size="xs"
          disabled={busy}
          aria-label={`Stop all for ${group.owner}`}
          onClick={() => onStop(group.owner)}
        >
          <CircleStop /> Stop All for Owner…
        </Button>
      </header>
      <ul className="pb-2">
        {group.runs.map((run) => {
          const ports = portEntries(run.ports)
            .map(([, p]) => p)
            .join(" · ");
          return (
            <li key={`${run.project}/${run.service}`}>
              <button
                type="button"
                onClick={() => onShow(run)}
                aria-label={`Show ${run.project}/${run.service}`}
                className="flex h-8 w-full items-center gap-2.5 px-6 text-left text-sm outline-none hover:bg-primary/8 focus-visible:bg-primary/12 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
              >
                <StatusGlyph style={statusStyle(run.state, run.health)} />
                <span className="min-w-0 flex-1 truncate text-ink">
                  {run.project}/{run.service}
                </span>
                {ports && <MetaLabel>{ports}</MetaLabel>}
                <Expiry run={run} />
              </button>
            </li>
          );
        })}
        {group.jobs.map((job) => (
          <li key={job.id} className="flex h-8 items-center gap-2.5 px-6 text-sm">
            <StatusGlyph style={statusStyle("starting")} />
            <span className="min-w-0 flex-1 truncate">
              {job.project} · {job.kind} {jobTitle(job)}
            </span>
            <MetaLabel>job</MetaLabel>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** What each owner (the user or an agent) left running, with a confirmed stop. */
export function OwnersPage() {
  const navigate = useNavigate();
  const runs = useRuns();
  const { jobs } = useJobs();
  const stop = useStopOwner();
  const [pending, setPending] = useState<string | null>(null);

  const groups = useMemo(() => ownerGroups(runs.data?.services ?? [], jobs), [runs.data, jobs]);
  const activeRuns = groups.reduce((n, g) => n + g.runs.length, 0);

  const show = (run: Run) => {
    useNav.getState().selectService({ project: run.project, service: run.service });
    useNav.getState().setInspectorOpen(true);
    void navigate(projectPath(run.project));
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <TopBar title="Owners" />
      <SectionHeader
        title="Owners"
        detail={`${groups.length} ${groups.length === 1 ? "owner" : "owners"} · ${activeRuns} active ${activeRuns === 1 ? "run" : "runs"}`}
      />
      <div className="hairline-t min-h-0 flex-1 overflow-auto bg-surface/60">
        {groups.length === 0 ? (
          <EmptyState
            icon={Users}
            title="Nothing running"
            description="Runs started by you or by agents (ROCKET_OWNER=agent:…) are grouped here."
          />
        ) : (
          groups.map((g) => (
            <GroupSection key={g.owner} group={g} busy={stop.isPending} onShow={show} onStop={setPending} />
          ))
        )}
      </div>

      <AlertDialog open={pending !== null} onOpenChange={(o) => !o && setPending(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Stop everything started by {pending}?</AlertDialogTitle>
            <AlertDialogDescription>
              Stops this owner&apos;s services in every project and cancels its running jobs.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (pending) stop.mutate(pending);
                setPending(null);
              }}
            >
              Stop All
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
