import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { applyJob, queryKeys } from "@/lib/cache";
import { describeError } from "@/lib/errors";
import { useLive } from "@/lib/live";
import { appendCapped } from "@/lib/logbuffer";
import { followJobLogs, type StopFollow } from "@/lib/rocket";

/** Lines kept in memory for the job log. */
export const JOB_LOG_CAP = 5000;
const FLUSH_MS = 60;

interface Buffer {
  key: string;
  lines: string[];
  ended: boolean;
  error: string | null;
}

const NO_LINES: string[] = [];

export interface JobLogs {
  lines: string[];
  /** The stream ended: the job reached a terminal state (or the daemon closed it). */
  ended: boolean;
  error: string | null;
  reload: () => void;
}

/**
 * Follows ONE job's log through `followJobLogs` (whole log, then live, then the
 * final state). The follow is the only source of lines: global `job.log`
 * events are ignored, so nothing is ever appended twice. It stops when the job
 * changes, `enabled` turns false, the component unmounts or the connection
 * drops, and restarts (replaying from the file) when the daemon is back.
 */
export function useJobLogs(jobId: string | null, enabled = true): JobLogs {
  const qc = useQueryClient();
  const online = useLive((s) => s.connection.state === "online");
  const [generation, setGeneration] = useState(0);
  // The buffer belongs to one follow; a different key (job, connection or
  // reload) reads as empty, because the new follow replays the whole log.
  const key = `${jobId}|${online}|${generation}`;
  const [state, setState] = useState<Buffer>({ key, lines: [], ended: false, error: null });
  const update = useCallback(
    (key: string, patch: (b: Buffer) => Partial<Buffer>) =>
      setState((prev) => {
        const base = prev.key === key ? prev : { key, lines: [], ended: false, error: null };
        return { ...base, ...patch(base) };
      }),
    [],
  );

  useEffect(() => {
    if (!jobId || !enabled || !online) return;

    let cancelled = false;
    let stop: StopFollow | undefined;
    let pending: string[] = [];
    let flushTimer: ReturnType<typeof setTimeout> | undefined;

    const flush = () => {
      flushTimer = undefined;
      if (cancelled || pending.length === 0) return;
      const batch = pending;
      pending = [];
      update(key, (b) => ({ lines: appendCapped(b.lines, batch, JOB_LOG_CAP) }));
    };

    followJobLogs(jobId, (m) => {
      if (cancelled) return;
      if (m.kind === "event") {
        const ev = m.event;
        if (ev.type === "job.log" && ev.line !== undefined) {
          pending.push(ev.line);
          flushTimer ??= setTimeout(flush, FLUSH_MS);
        } else if (ev.type === "job.state" && ev.job) {
          applyJob(qc, ev.job);
        }
      } else if (m.kind === "end") {
        if (flushTimer) clearTimeout(flushTimer);
        flush();
        update(key, () => ({ ended: true }));
        void qc.invalidateQueries({ queryKey: queryKeys.jobs });
      } else {
        update(key, () => ({ error: m.message }));
      }
    }).then(
      (s) => {
        if (cancelled) void s();
        else stop = s;
      },
      (e) => {
        if (!cancelled) update(key, () => ({ error: describeError(e).detail ?? describeError(e).title }));
      },
    );

    return () => {
      cancelled = true;
      if (flushTimer) clearTimeout(flushTimer);
      void stop?.();
    };
  }, [jobId, enabled, online, key, qc, update]);

  const reload = useCallback(() => setGeneration((g) => g + 1), []);
  const current = state.key === key ? state : null;
  return {
    lines: current?.lines ?? NO_LINES,
    ended: current?.ended ?? false,
    error: current?.error ?? null,
    reload,
  };
}
