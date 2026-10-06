import { useCallback, useEffect, useState } from "react";
import { describeError } from "@/lib/errors";
import { appendCapped } from "@/lib/logbuffer";
import { followServiceLogs, type StopFollow } from "@/lib/rocket";

/** Lines kept in memory for the inspector tail. */
export const LOG_CAP = 2000;
const TAIL = 300;
const FLUSH_MS = 60;

export interface ServiceLogs {
  lines: string[];
  error: string | null;
  /** Restarts the follow: re-reads the tail and keeps streaming. */
  reload: () => void;
}

/**
 * Follows one service's log (tail, then live). The follow is stopped when the
 * service changes, `enabled` turns false or the component unmounts. Lines are
 * batched so a chatty service does not re-render once per line.
 */
export function useServiceLogs(project: string | undefined, service: string | undefined, enabled: boolean): ServiceLogs {
  const [lines, setLines] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    if (!enabled || !project || !service) return;
    let cancelled = false;
    let stop: StopFollow | undefined;
    let pending: string[] = [];
    let flushTimer: ReturnType<typeof setTimeout> | undefined;

    const flush = () => {
      flushTimer = undefined;
      if (cancelled || pending.length === 0) return;
      const batch = pending;
      pending = [];
      setLines((prev) => appendCapped(prev, batch, LOG_CAP));
    };

    followServiceLogs(
      project,
      service,
      (m) => {
        if (cancelled) return;
        if (m.kind === "event") {
          if (m.event.type === "log.line" && m.event.line !== undefined) {
            pending.push(m.event.line);
            flushTimer ??= setTimeout(flush, FLUSH_MS);
          }
        } else if (m.kind === "error") {
          setError(m.message);
        }
      },
      TAIL,
    ).then(
      (s) => {
        if (cancelled) void s();
        else stop = s;
      },
      (e) => {
        if (!cancelled) setError(describeError(e).detail ?? describeError(e).title);
      },
    );

    return () => {
      cancelled = true;
      if (flushTimer) clearTimeout(flushTimer);
      void stop?.();
    };
  }, [project, service, enabled, generation]);

  const reload = useCallback(() => {
    setLines([]);
    setError(null);
    setGeneration((g) => g + 1);
  }, []);

  return { lines, error, reload };
}
