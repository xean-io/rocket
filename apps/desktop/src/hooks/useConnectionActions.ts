import { useIsFetching, useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "sonner";
import { describeError } from "@/lib/errors";
import { useLive } from "@/lib/live";
import { rocket } from "@/lib/rocket";

/** Asks the backend supervisor to retry the daemon connection now. */
export function useReconnect() {
  const m = useMutation({
    mutationFn: rocket.reconnect,
    onError: (e) => {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail });
    },
  });
  return { reconnect: () => m.mutate(), pending: m.isPending };
}

/** Reloads every snapshot; reconnects first when the daemon is offline. */
export function useRefresh() {
  const qc = useQueryClient();
  const { reconnect } = useReconnect();
  const fetching = useIsFetching() > 0;
  const refresh = useCallback(() => {
    if (useLive.getState().connection.state !== "online") reconnect();
    void qc.invalidateQueries();
  }, [qc, reconnect]);
  return { refresh, fetching };
}
