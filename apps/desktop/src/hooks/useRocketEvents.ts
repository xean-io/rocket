import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ConnectionStatus, Event } from "@/lib/bindings";
import { applyEvent, resync } from "@/lib/cache";
import { useLive } from "@/lib/live";
import { EVENTS, rocket } from "@/lib/rocket";

/**
 * Wires the backend's `rocket://event|connection|resync` into the react-query
 * cache and the live store. Mount once, near the root.
 */
export function useRocketEvents(): void {
  const qc = useQueryClient();
  const setConnection = useLive((s) => s.setConnection);

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const track = (p: Promise<UnlistenFn>) =>
      p.then((un) => (disposed ? un() : unlisteners.push(un)));

    track(listen<Event>(EVENTS.event, (e) => applyEvent(qc, e.payload)));
    track(listen<ConnectionStatus>(EVENTS.connection, (e) => setConnection(e.payload)));
    track(listen(EVENTS.resync, () => resync(qc)));
    // The supervisor may have connected before the listeners existed.
    rocket.connectionStatus().then(setConnection, () => {});

    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [qc, setConnection]);
}
