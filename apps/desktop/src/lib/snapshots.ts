// Stale-snapshot protection (port of the Swift `jobsRevision` idea): events
// patch cached snapshots in place and advance a per-key revision. A snapshot
// request remembers the revision it started under; if an event landed while it
// was in flight, the response is older than the cache and must not overwrite it.
import type { QueryClient, QueryKey } from "@tanstack/react-query";

const revisions = new WeakMap<QueryClient, Map<string, number>>();

const id = (key: QueryKey) => JSON.stringify(key);

function table(qc: QueryClient): Map<string, number> {
  let t = revisions.get(qc);
  if (!t) revisions.set(qc, (t = new Map()));
  return t;
}

export function currentRevision(qc: QueryClient, key: QueryKey): number {
  return table(qc).get(id(key)) ?? 0;
}

/** Marks that newer-than-any-in-flight-snapshot state was applied to `key`. */
export function bumpRevision(qc: QueryClient, key: QueryKey): void {
  const t = table(qc);
  t.set(id(key), (t.get(id(key)) ?? 0) + 1);
}

/**
 * Runs `fetcher`; when an event touched `key` meanwhile, returns the cached
 * (event-applied) value instead of the older response.
 */
export async function guardedFetch<T>(
  qc: QueryClient,
  key: QueryKey,
  fetcher: () => Promise<T>,
): Promise<T> {
  const started = currentRevision(qc, key);
  const data = await fetcher();
  if (currentRevision(qc, key) === started) return data;
  return qc.getQueryData<T>(key) ?? data;
}
