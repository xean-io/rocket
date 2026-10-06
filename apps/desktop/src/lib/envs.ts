// Environment selection logic (port of RocketStore.knownEnvs/validatedEnv).
import type { ProjectInfo, Run } from "./bindings";

/**
 * Declared environments are authoritative, including an explicitly empty list.
 * Older daemons omit `envs`: fall back to the default plus envs seen on runs.
 */
export function knownEnvs(info: ProjectInfo | null | undefined, runs: readonly Run[]): string[] {
  if (!info) return [];
  if (info.envs != null) return [...info.envs].sort();
  const set = new Set(runs.map((r) => r.env).filter(Boolean));
  if (info.default_env) set.add(info.default_env);
  return [...set].sort();
}

/** An explicit selection only counts while it is still a known env. */
export function validatedEnv(selection: string | undefined, known: readonly string[]): string | undefined {
  return selection !== undefined && known.includes(selection) ? selection : undefined;
}

/**
 * Drops selections whose env no longer exists. Projects whose options are not
 * loaded yet are left alone. Returns the same object when nothing changed.
 */
export function reconcileEnvSelections(
  selections: Record<string, string>,
  known: Record<string, readonly string[]>,
): Record<string, string> {
  let changed = false;
  const next: Record<string, string> = {};
  for (const [project, env] of Object.entries(selections)) {
    const options = known[project];
    if (options && validatedEnv(env, options) === undefined) {
      changed = true;
      continue;
    }
    next[project] = env;
  }
  return changed ? next : selections;
}
