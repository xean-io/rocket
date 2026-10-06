import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { useMenuActions } from "@/hooks/useMenuActions";
import { useLive } from "@/lib/live";
import { rocket } from "@/lib/rocket";
import { checkNow, checkQuietly, useUpdateStore } from "@/lib/updateRuntime";
import { startScheduler } from "@/lib/updates";

/** Version info that never changes while the app runs. */
export function useUpdaterEnv() {
  return useQuery({ queryKey: ["updater-env"], queryFn: rocket.updaterEnv, staleTime: Infinity });
}

/**
 * Mount once: schedules the automatic checks (not in debug builds) and handles
 * "Check for Updates…" from the app menu and the tray.
 */
export function useUpdates(): void {
  const env = useUpdaterEnv();
  const autoAllowed = env.data?.auto_check ?? false;
  useEffect(() => {
    if (!autoAllowed) return;
    return startScheduler({ run: checkQuietly, enabled: () => useUpdateStore.getState().autoCheck });
  }, [autoAllowed]);
  useMenuActions({ "app.check_updates": () => void checkNow() });
}

/** The daemon when it is older than the bundled CLI it was started from. */
export function useStaleDaemon() {
  const connection = useLive((s) => s.connection.state);
  return useQuery({
    // A reconnect after a restart means fresh daemon facts.
    queryKey: ["daemon-stale", connection],
    queryFn: rocket.daemonStale,
    enabled: connection === "online",
  });
}
