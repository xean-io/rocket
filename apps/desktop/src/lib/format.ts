// Display formatting.

export const parseTime = (iso?: string | null): number | undefined => {
  if (!iso) return undefined;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? undefined : t;
};

/** Sorted `[name, port]` pairs; empty when there are none. */
export function portEntries(ports?: { [key in string]: number } | null): [string, number][] {
  return Object.entries(ports ?? {}).sort(([a], [b]) => a.localeCompare(b));
}

/** `1h 4m`, `3m 12s`, `8s`: at most two units. */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

/** `m:ss` or `h:mm:ss` remaining time. */
export function formatCountdown(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

export function formatClock(iso?: string | null): string | undefined {
  const t = parseTime(iso);
  if (t === undefined) return undefined;
  return new Date(t).toLocaleTimeString(undefined, { hour12: false });
}

export const formatOwner = (owner?: string) => owner || "user";

/** `just now`, `3 minutes ago`, `yesterday`: coarse relative time like a platform relative formatter. */
export function formatRelative(iso: string | undefined | null, now: number = Date.now()): string {
  const t = parseTime(iso);
  if (t === undefined || t <= 0) return "—";
  const seconds = Math.round((t - now) / 1000);
  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 45) return "just now";
  if (abs < 3600) return rtf.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(seconds / 3600), "hour");
  return rtf.format(Math.round(seconds / 86400), "day");
}
