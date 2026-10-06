import type { GcResult } from "./bindings";

/** One toast line for a garbage-collection run, e.g. `2 pruned, 1 lost_job`. */
export function gcSummary(result: GcResult): string {
  if (result.actions.length === 0) return "Nothing to clean up";
  const counts = new Map<string, number>();
  for (const a of result.actions) counts.set(a.action, (counts.get(a.action) ?? 0) + 1);
  const parts = [...counts].map(([action, n]) => `${n} ${action.replaceAll("_", " ")}`);
  const n = result.actions.length;
  return `Cleaned ${n} ${n === 1 ? "item" : "items"}: ${parts.join(", ")}`;
}
