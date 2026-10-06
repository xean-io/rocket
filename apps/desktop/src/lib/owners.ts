// Owner grouping (port of RocketStore.ownerGroups + OwnersView).
import type { Job, Run } from "./bindings";
import { formatOwner } from "./format";
import { isActive, isAgentOwned } from "./status";

export interface OwnerGroup {
  owner: string;
  isAgent: boolean;
  runs: Run[];
  jobs: Job[];
}

/**
 * Active runs and running jobs grouped by owner (empty owner means `user`),
 * sorted by owner. An owner with only a running job is listed too: stopping
 * the owner cancels that job.
 */
export function ownerGroups(runs: readonly Run[], jobs: readonly Job[]): OwnerGroup[] {
  const groups = new Map<string, OwnerGroup>();
  const group = (owner: string) => {
    let g = groups.get(owner);
    if (!g) groups.set(owner, (g = { owner, isAgent: isAgentOwned(owner), runs: [], jobs: [] }));
    return g;
  };
  for (const r of runs) if (isActive(r.state)) group(formatOwner(r.owner)).runs.push(r);
  for (const j of jobs) if (j.status === "running") group(formatOwner(j.owner)).jobs.push(j);
  const key = (r: Run) => `${r.project}/${r.service}`;
  for (const g of groups.values()) g.runs.sort((a, b) => key(a).localeCompare(key(b)));
  return [...groups.values()].sort((a, b) => a.owner.localeCompare(b.owner));
}

export const ownerCount = (runs: readonly Run[], jobs: readonly Job[]) => ownerGroups(runs, jobs).length;
