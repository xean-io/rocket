import { Loader2, ZapOff } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { Button } from "@/components/ui/button";
import { useReconnect } from "@/hooks/useConnectionActions";
import { useLive } from "@/lib/live";
import { cn } from "@/lib/utils";

/** Daemon connection pill in the title strip. */
export function ConnectionBadge() {
  const connection = useLive((s) => s.connection);
  const view = {
    online: { text: "Live", dot: "bg-status-ok", pulse: false },
    connecting: { text: "Connecting", dot: "bg-status-warn", pulse: true },
    offline: { text: "Offline", dot: "bg-status-bad", pulse: false },
  }[connection.state];
  return (
    <span
      role="status"
      aria-label={`Daemon ${view.text.toLowerCase()}`}
      title={connection.state === "online" ? `rocketd ${connection.version} (pid ${connection.pid})` : undefined}
      className="meta flex items-center gap-1.5 rounded-full border border-border bg-surface/70 px-2 py-0.5"
    >
      <span className={cn("size-1.5 rounded-full", view.dot, view.pulse && "status-pulse")} />
      {view.text}
    </span>
  );
}

/** Whole-pane state while there is nothing cached to show. */
export function ConnectionStateView() {
  const connection = useLive((s) => s.connection);
  const { reconnect, pending } = useReconnect();
  if (connection.state === "offline") {
    return (
      <EmptyState
        icon={ZapOff}
        title="rocketd is not reachable"
        description={<span className="selectable break-words font-mono text-xs">{connection.reason}</span>}
      >
        <Button onClick={reconnect} disabled={pending}>
          {pending && <Loader2 className="animate-spin" />}
          Reconnect
        </Button>
      </EmptyState>
    );
  }
  return (
    <EmptyState icon={Loader2} title="Connecting to rocketd…" description="Starting the daemon if it is not running yet." className="[&_svg]:animate-spin" />
  );
}

/** Slim strip at the bottom of the pane while stale data is on screen. */
export function ConnectionStrip() {
  const connection = useLive((s) => s.connection);
  const { reconnect, pending } = useReconnect();
  if (connection.state === "online") return null;
  const offline = connection.state === "offline";
  return (
    <div
      role="alert"
      className={cn(
        "hairline-t flex h-9 shrink-0 items-center gap-3 bg-surface px-4 text-sm",
        offline ? "text-status-bad" : "text-status-warn",
      )}
    >
      {offline ? <ZapOff className="size-4" /> : <Loader2 className="size-4 animate-spin" />}
      <span className="min-w-0 flex-1 truncate">
        {offline ? `rocketd is offline. Showing the last known state. ${connection.reason}` : "Connecting to rocketd…"}
      </span>
      {offline && (
        <Button size="xs" variant="outline" onClick={reconnect} disabled={pending}>
          Reconnect
        </Button>
      )}
    </div>
  );
}
