// react-query hooks over the daemon snapshots. Queries wait for `online`;
// event-applied cache patches are protected from older in-flight responses.
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import type { Conflict, Run, Summary } from "./bindings";
import { queryKeys } from "./cache";
import { useLive } from "./live";
import { RocketError, rocket } from "./rocket";
import { fetchJobsGuarded } from "./jobs";
import { guardedFetch } from "./snapshots";

export const useIsOnline = () => useLive((s) => s.connection.state === "online");

export function useProjects() {
  const online = useIsOnline();
  return useQuery({ queryKey: queryKeys.projects, queryFn: rocket.projects, enabled: online });
}

/** Every known run across projects (`ps --all`). */
export function useRuns() {
  const qc = useQueryClient();
  const online = useIsOnline();
  return useQuery({
    queryKey: queryKeys.runs,
    queryFn: () => guardedFetch(qc, queryKeys.runs, () => rocket.ps(null, true)),
    enabled: online,
  });
}

/** Older daemons without `/v1/status`: `ps` still lists declared services. */
async function fetchSummary(project: string): Promise<Summary> {
  try {
    return await rocket.status(project);
  } catch (e) {
    if (e instanceof RocketError && e.code === "endpoint") {
      const ps = await rocket.ps(project);
      return { project: null, services: ps.services, jobs: [], conflicts: [] };
    }
    throw e;
  }
}

export function useProjectSummary(project: string | undefined) {
  const qc = useQueryClient();
  const online = useIsOnline();
  const key = queryKeys.projectSummary(project ?? "");
  return useQuery({
    queryKey: key,
    queryFn: () => guardedFetch(qc, key, () => fetchSummary(project as string)),
    enabled: online && !!project,
  });
}

/** Leased ports across projects (`GET /v1/ports`); events invalidate it. */
export function usePorts() {
  const online = useIsOnline();
  return useQuery({ queryKey: queryKeys.ports, queryFn: rocket.ports, enabled: online });
}

/** Port conflicts (remaps and blocked ports) of every given project. */
export function useConflicts(projects: readonly string[]): Conflict[] {
  const qc = useQueryClient();
  const online = useIsOnline();
  const results = useQueries({
    queries: projects.map((project) => {
      const key = queryKeys.projectSummary(project);
      return {
        queryKey: key,
        queryFn: () => guardedFetch(qc, key, () => fetchSummary(project)),
        enabled: online,
      };
    }),
  });
  return results.flatMap((r) => r.data?.conflicts ?? []);
}

/**
 * Recent jobs, newest first. `unavailable` is set for daemons without the
 * jobs API (`jobsAvailable`); events patch the cache in place.
 */
export function useJobs() {
  const qc = useQueryClient();
  const online = useIsOnline();
  const query = useQuery({
    queryKey: queryKeys.jobs,
    queryFn: () => fetchJobsGuarded(qc, () => rocket.jobs({ all: true, limit: 50 })),
    enabled: online,
  });
  const unavailable = query.error instanceof RocketError && query.error.code === "endpoint";
  return { ...query, jobs: query.data?.jobs ?? [], unavailable };
}

/** Registered projects plus any project with known runs, sorted. */
export function useProjectNames(): { names: string[]; loading: boolean } {
  const projects = useProjects();
  const runs = useRuns();
  const set = new Set<string>(projects.data?.projects.map((p) => p.name));
  for (const r of runs.data?.services ?? []) set.add(r.project);
  return { names: [...set].sort(), loading: projects.isPending };
}

export function sortedServices(runs: readonly Run[]): Run[] {
  return [...runs].sort((a, b) => a.service.localeCompare(b.service));
}
