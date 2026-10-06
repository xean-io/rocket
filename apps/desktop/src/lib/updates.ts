// Auto-update logic that does not touch Tauri or the DOM: persisted settings,
// the "Later" suppression, and the launch + periodic check schedule. The
// wiring (toasts, commands, store) lives in `updateRuntime.ts`.
import type { UpdateStatus } from "./bindings";

/** First automatic check after launch. */
export const FIRST_CHECK_DELAY_MS = 10_000;
/** Automatic checks while the app keeps running. */
export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

export const AUTO_CHECK_KEY = "rocket.updates.autoCheck";
export const LAST_CHECKED_KEY = "rocket.updates.lastChecked";

type KeyValueStore = Pick<Storage, "getItem" | "setItem">;

/** `localStorage` can be missing or throw (blocked data, private windows). */
function defaultStore(): KeyValueStore | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

/** "Check for updates automatically": on unless the user turned it off. */
export function readAutoCheck(store: KeyValueStore | null = defaultStore()): boolean {
  try {
    return store?.getItem(AUTO_CHECK_KEY) !== "false";
  } catch {
    return true;
  }
}

export function writeAutoCheck(on: boolean, store: KeyValueStore | null = defaultStore()): void {
  try {
    store?.setItem(AUTO_CHECK_KEY, String(on));
  } catch {
    // The setting then lasts until the app quits.
  }
}

/** Epoch milliseconds of the last completed check, if known. */
export function readLastChecked(store: KeyValueStore | null = defaultStore()): number | null {
  try {
    const n = Number(store?.getItem(LAST_CHECKED_KEY));
    return Number.isFinite(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
}

export function writeLastChecked(at: number, store: KeyValueStore | null = defaultStore()): void {
  try {
    store?.setItem(LAST_CHECKED_KEY, String(at));
  } catch {
    // Display-only; losing it is harmless.
  }
}

/**
 * Runs `run` once after `firstDelayMs`, then every `intervalMs`, skipping ticks
 * while `enabled()` is false. Returns a function that cancels everything.
 */
export function startScheduler(opts: {
  run: () => void | Promise<void>;
  enabled: () => boolean;
  firstDelayMs?: number;
  intervalMs?: number;
}): () => void {
  const { run, enabled, firstDelayMs = FIRST_CHECK_DELAY_MS, intervalMs = CHECK_INTERVAL_MS } = opts;
  let interval: ReturnType<typeof setInterval> | undefined;
  const tick = () => {
    if (enabled()) void run();
  };
  const first = setTimeout(() => {
    tick();
    interval = setInterval(tick, intervalMs);
  }, firstDelayMs);
  return () => {
    clearTimeout(first);
    if (interval !== undefined) clearInterval(interval);
  };
}

export interface UpdateUi {
  /** An update is available (and was not postponed with "Later"). */
  announce(status: UpdateStatus): void;
  /** A manual check found nothing newer. */
  upToDate(status: UpdateStatus): void;
  /** A manual check failed. Automatic checks never call this. */
  failed(error: unknown): void;
}

export interface UpdateControllerDeps {
  check(): Promise<UpdateStatus>;
  ui: UpdateUi;
  /** Called with the time of every completed check. */
  onChecked?(at: number): void;
  now?: () => number;
}

/**
 * Decides what a check result means for the user. Automatic checks are quiet:
 * no "up to date" message, no errors (offline is the common case), and a
 * version the user postponed stays hidden until the next launch. Manual checks
 * always answer.
 */
export function createUpdateController(deps: UpdateControllerDeps) {
  const { check, ui, onChecked, now = Date.now } = deps;
  const postponed = new Set<string>();
  let running = false;

  return {
    /** "Later": hide this version from automatic checks until the next launch. */
    later(version: string) {
      postponed.add(version);
    },
    isPostponed: (version: string) => postponed.has(version),
    async check({ manual }: { manual: boolean }): Promise<void> {
      if (running) return;
      running = true;
      try {
        const status = await check();
        onChecked?.(now());
        if (status.available && status.version) {
          if (manual || !postponed.has(status.version)) ui.announce(status);
        } else if (manual) {
          ui.upToDate(status);
        }
      } catch (e) {
        if (manual) ui.failed(e);
      } finally {
        running = false;
      }
    },
  };
}

/** Download progress for the toast: `42%`, or the size so far when unknown. */
export function progressLabel(received: number, total: number | null): string {
  if (total && total > 0) return `${Math.min(100, Math.floor((received / total) * 100))}%`;
  if (received <= 0) return "Starting…";
  return `${(received / (1024 * 1024)).toFixed(1)} MB`;
}

/** Shown when an update replaced the bundled CLI but the old daemon still runs. */
export function staleDaemonMessage(stale: { running: string; bundled: string }): string {
  return `Daemon is running ${stale.running}; restart it to use ${stale.bundled}.`;
}
