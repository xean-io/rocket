// Visual vocabulary for run and job states (port of Support/StatusStyle.swift).
import type { Health, JobStatus, Run, RunState } from "./bindings";

export type Tone = "ok" | "warn" | "bad" | "muted";
export type Glyph = "dot" | "ring" | "dotted" | "alert" | "x" | "bolt";

export interface StatusStyle {
  tone: Tone;
  glyph: Glyph;
  label: string;
  pulses: boolean;
}

export function statusStyle(state: RunState, health: Health = "unknown"): StatusStyle {
  switch (state) {
    case "running":
      return health === "unhealthy"
        ? { tone: "warn", glyph: "alert", label: "Unhealthy", pulses: false }
        : { tone: "ok", glyph: "dot", label: "Running", pulses: false };
    case "starting":
      return { tone: "warn", glyph: "dotted", label: "Starting", pulses: true };
    case "stopping":
      return { tone: "muted", glyph: "dotted", label: "Stopping", pulses: true };
    case "stopped":
      return { tone: "muted", glyph: "ring", label: "Stopped", pulses: false };
    case "exited":
      return { tone: "warn", glyph: "x", label: "Exited", pulses: false };
    case "failed":
      return { tone: "bad", glyph: "alert", label: "Failed", pulses: false };
    case "dead":
      return { tone: "bad", glyph: "bolt", label: "Dead", pulses: false };
  }
}

export function jobStatusStyle(status: JobStatus): StatusStyle {
  switch (status) {
    case "running":
      return { tone: "warn", glyph: "dotted", label: "Running", pulses: true };
    case "succeeded":
      return { tone: "ok", glyph: "dot", label: "Succeeded", pulses: false };
    case "failed":
      return { tone: "bad", glyph: "alert", label: "Failed", pulses: false };
    case "canceled":
      return { tone: "muted", glyph: "ring", label: "Canceled", pulses: false };
    case "lost":
      return { tone: "warn", glyph: "alert", label: "Lost", pulses: false };
  }
}

/** Starting, running or stopping: the run still owns processes. */
export function isActive(state: RunState): boolean {
  return state === "starting" || state === "running" || state === "stopping";
}

export interface StatusCounts {
  running: number;
  changing: number;
  down: number;
}

/** Header capsule counts: running, starting/stopping, failed/dead/exited. */
export function statusCounts(runs: readonly Run[]): StatusCounts {
  const counts: StatusCounts = { running: 0, changing: 0, down: 0 };
  for (const r of runs) {
    if (r.state === "running") counts.running++;
    else if (r.state === "starting" || r.state === "stopping") counts.changing++;
    else if (r.state === "failed" || r.state === "dead" || r.state === "exited") counts.down++;
  }
  return counts;
}

export const isAgentOwned = (owner?: string) => owner?.startsWith("agent:") ?? false;
