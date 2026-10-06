// Job helpers.
import type { QueryClient } from "@tanstack/react-query";
import type { Job, JobRequest, JobsResult, JobStatus } from "./bindings";
import { queryKeys } from "./cache";
import { currentRevision } from "./snapshots";

/** Anything but `running` is final: a job never goes back to running. */
export const isTerminal = (status: JobStatus) => status !== "running";

export const jobTitle = (job: Pick<Job, "name" | "kind">) => job.name || job.kind;

export const runningJobs = (jobs: readonly Job[]) => jobs.filter((j) => j.status === "running");

const startedAt = (j: Job) => Date.parse(j.started_at) || 0;

/** Newest first; ties broken by id (descending) so the order is stable. */
export function sortJobs(jobs: readonly Job[]): Job[] {
  return [...jobs].sort((a, b) => {
    const d = startedAt(b) - startedAt(a);
    if (d !== 0) return d;
    return a.id < b.id ? 1 : a.id > b.id ? -1 : 0;
  });
}

/**
 * A terminal SSE event can arrive before the POST's older "running" response,
 * and a list snapshot can be older than an event: keep the terminal job.
 */
export function retainTerminal(current: Job | undefined, incoming: Job): Job {
  if (current && isTerminal(current.status) && !isTerminal(incoming.status)) return current;
  return incoming;
}

/** The incoming list (a snapshot), keeping terminal jobs the cache already knows. */
export function mergeJobs(current: readonly Job[], incoming: readonly Job[]): Job[] {
  const byId = new Map(current.map((j) => [j.id, j]));
  return sortJobs(incoming.map((j) => retainTerminal(byId.get(j.id), j)));
}

/**
 * Runs the jobs fetcher. When an event patched the cache meanwhile the response
 * is older than the cache and is dropped; otherwise it is merged so a finished
 * job is never shown as running again (`jobsRevision` + terminal rule).
 */
export async function fetchJobsGuarded(
  qc: QueryClient,
  fetcher: () => Promise<JobsResult>,
): Promise<JobsResult> {
  const started = currentRevision(qc, queryKeys.jobs);
  const response = await fetcher();
  const cached = qc.getQueryData<JobsResult>(queryKeys.jobs);
  if (currentRevision(qc, queryKeys.jobs) !== started) return cached ?? response;
  return { ...response, jobs: mergeJobs(cached?.jobs ?? [], response.jobs) };
}

const nonEmpty = <T>(list: T[] | undefined): T[] | undefined => (list && list.length > 0 ? list : undefined);

/**
 * The request that starts `job` again, preserving env, profiles and args
 * (rerun requests). Deploys target their env and carry no
 * profiles; they still need the human's explicit yes (see `deploy.ts`).
 */
export function rerunRequest(job: Job): JobRequest {
  const deploy = job.kind === "deploy";
  const env = deploy ? job.env || job.name : job.env || undefined;
  const req: JobRequest = {
    project: job.project,
    kind: job.kind,
    name: deploy ? (env ?? job.name) : job.name,
  };
  if (env) req.env = env;
  const profiles = deploy ? undefined : nonEmpty(job.profiles);
  if (profiles) req.profiles = profiles;
  const args = nonEmpty(job.args);
  if (args) req.args = args;
  return req;
}

export type StepState = "done" | "current" | "failed" | "pending";

/** Progress marker of the `index`-th (0-based) step of `job`. */
export function stepState(job: Job, index: number): StepState {
  if (job.status === "succeeded") return "done";
  const current = (job.step ?? 0) - 1;
  if (index < current) return "done";
  if (index === current) {
    if (job.status === "running") return "current";
    if (job.status === "failed" || job.status === "lost") return "failed";
    return "pending";
  }
  return "pending";
}

export function stepLabel(step: { task?: string; run?: string }): string {
  return step.task ? `task ${step.task}` : `run ${step.run ?? ""}`;
}

/** Elapsed time: live for a running job, the recorded duration once it ended. */
export function jobDuration(job: Job, now: number): number {
  if (job.status !== "running") return job.duration_ms;
  const started = Date.parse(job.started_at);
  return Number.isNaN(started) || started <= 0 ? job.duration_ms : Math.max(0, now - started);
}
