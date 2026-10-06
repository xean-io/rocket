// Daemon events -> react-query cache. Pure helpers plus `applyEvent`.
import type { QueryClient } from "@tanstack/react-query";
import type { Event, Job, JobsResult, Run, StatusResult, Summary } from "./bindings";
import { retainTerminal, sortJobs } from "./jobs";
import { bumpRevision } from "./snapshots";

export const queryKeys = {
  health: ["health"] as const,
  projects: ["projects"] as const,
  /** Every known run (`ps --all`); `service.state` events patch it in place. */
  runs: ["runs"] as const,
  summary: ["summary"] as const,
  /** One project's declared services, envs, pipelines and conflicts. */
  projectSummary: (project: string) => ["summary", project] as const,
  ports: ["ports"] as const,
  /** Recent jobs; `job.state` events patch it in place. */
  jobs: ["jobs"] as const,
  daemon: ["daemon"] as const,
};

const sameRun = (a: Run, b: Run) => a.project === b.project && a.service === b.service;

/** Replaces the run with the same project/service, or appends it. */
export function upsertRun(runs: Run[], run: Run): Run[] {
  const i = runs.findIndex((r) => sameRun(r, run));
  if (i === -1) return [...runs, run];
  const next = runs.slice();
  next[i] = run;
  return next;
}

/**
 * Replaces the job with the same id, or adds it; newest first. A terminal job
 * is never overwritten by a late "running" snapshot (`retainingTerminalJob`).
 */
export function upsertJob(jobs: Job[], job: Job): Job[] {
  const current = jobs.find((j) => j.id === job.id);
  const next = retainTerminal(current, job);
  return sortJobs([...jobs.filter((j) => j.id !== job.id), next]);
}

/** Patches only the status of a cached job (a `job.state` event without the job). */
function patchJobStatus(jobs: Job[], id: string, status: Job["status"]): Job[] {
  const current = jobs.find((j) => j.id === id);
  if (!current) return jobs;
  return upsertJob(jobs, { ...current, status });
}

/** Applies a job snapshot (event or action result) to the cached jobs list. */
export function applyJob(qc: QueryClient, job: Job): void {
  qc.setQueryData<JobsResult>(queryKeys.jobs, (old) =>
    old ? { ...old, jobs: upsertJob(old.jobs, job) } : old,
  );
  bumpRevision(qc, queryKeys.jobs);
}

/**
 * Applies one daemon event to the cache: state events patch the cached
 * snapshots in place, everything derived from them is invalidated. Log events
 * are not cached (they flow through the follow channels).
 */
export function applyEvent(qc: QueryClient, ev: Event): void {
  switch (ev.type) {
    case "service.state":
      if (ev.run) {
        const run = ev.run;
        qc.setQueryData<StatusResult>(queryKeys.runs, (old) =>
          old ? { ...old, services: upsertRun(old.services, run) } : old,
        );
        bumpRevision(qc, queryKeys.runs);
        const summaryKey = queryKeys.projectSummary(run.project);
        qc.setQueryData<Summary>(summaryKey, (old) =>
          old ? { ...old, services: upsertRun(old.services, run) } : old,
        );
        bumpRevision(qc, summaryKey);
      }
      void qc.invalidateQueries({ queryKey: queryKeys.summary });
      void qc.invalidateQueries({ queryKey: queryKeys.ports });
      break;
    case "job.state":
      if (ev.job) {
        applyJob(qc, ev.job);
      } else if (ev.job_id && ev.status) {
        const { job_id: id, status } = ev;
        qc.setQueryData<JobsResult>(queryKeys.jobs, (old) =>
          old ? { ...old, jobs: patchJobStatus(old.jobs, id, status) } : old,
        );
        bumpRevision(qc, queryKeys.jobs);
      }
      void qc.invalidateQueries({ queryKey: queryKeys.summary });
      break;
    case "port.leased":
    case "port.released":
      void qc.invalidateQueries({ queryKey: queryKeys.ports });
      break;
    default:
      break;
  }
}

/** The stream reopened: any cached snapshot may be stale. */
export function resync(qc: QueryClient): void {
  void qc.invalidateQueries();
}
