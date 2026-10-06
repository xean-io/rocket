import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Download, FolderOpen, Loader2, PlugZap, RefreshCw, RotateCcw, Trash2, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { MetaLabel } from "@/components/MetaLabel";
import { SectionHeader } from "@/components/SectionHeader";
import { ConnectionBadge } from "@/components/shell/ConnectionViews";
import { TopBar } from "@/components/shell/TopBar";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useReconnect } from "@/hooks/useConnectionActions";
import { useGc } from "@/hooks/useGc";
import { useStaleDaemon, useUpdaterEnv } from "@/hooks/useUpdates";
import { queryKeys } from "@/lib/cache";
import { describeError } from "@/lib/errors";
import { useLive } from "@/lib/live";
import { rocket } from "@/lib/rocket";
import { checkNow, installNow, useUpdateStore } from "@/lib/updateRuntime";
import { staleDaemonMessage } from "@/lib/updates";
import { cn } from "@/lib/utils";

function Row({ label, value, mono = true }: { label: string; value?: React.ReactNode; mono?: boolean }) {
  if (value === undefined || value === null || value === "") return null;
  return (
    <div className="grid grid-cols-[8.5rem_1fr] items-baseline gap-x-4 py-1.5">
      <MetaLabel>{label}</MetaLabel>
      <span className={cn("selectable min-w-0 break-all text-sm text-ink", mono && "font-mono text-[0.82rem]")}>
        {value}
      </span>
    </div>
  );
}

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section aria-label={title} className="hairline-b px-6 py-3.5">
      <h3 className="meta mb-1.5">{title}</h3>
      {children}
    </section>
  );
}

const formatStarted = (iso?: string) => {
  const t = iso ? Date.parse(iso) : NaN;
  return Number.isNaN(t) || t <= 0 ? undefined : new Date(t).toLocaleString();
};

/** Daemon facts and controls. The TCP token is never shown. */
export function SettingsPage() {
  const qc = useQueryClient();
  const connection = useLive((s) => s.connection);
  const info = useQuery({
    // A new connection state means new daemon facts (pid, version).
    queryKey: [...queryKeys.daemon, connection.state],
    queryFn: rocket.daemonInfo,
  });
  const { reconnect, pending: reconnecting } = useReconnect();
  const gc = useGc();
  const [confirmRestart, setConfirmRestart] = useState(false);
  const stale = useStaleDaemon().data;
  const env = useUpdaterEnv().data;
  const updates = useUpdateStore();

  const restart = useMutation({
    mutationFn: rocket.restartDaemon,
    onSuccess: () => toast.success("rocketd restarted", { description: "Running services were adopted by the new daemon." }),
    onError: (e) => {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail });
    },
    onSettled: () => qc.invalidateQueries(),
  });

  const d = info.data;
  const health = d?.health;
  const reveal = (path: string) => {
    revealItemInDir(path).catch((e) => toast.error("Could not open Finder", { description: String(e) }));
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <TopBar title="Settings" />
      <div className="min-h-0 flex-1 overflow-auto bg-surface/60">
        <div className="max-w-3xl">
          <SectionHeader title="Settings" detail="rocketd, locations and maintenance" />

          <Group title="Daemon">
            <Row
              label="Status"
              mono={false}
              value={
                <span className="flex flex-wrap items-center gap-2">
                  <ConnectionBadge />
                  {connection.state === "offline" && <span className="text-status-bad">{connection.reason}</span>}
                </span>
              }
            />
            <Row label="Version" value={health?.version} />
            <Row label="API" value={health?.api} />
            <Row label="PID" value={health?.pid} />
            <Row label="Started" mono={false} value={formatStarted(health?.started_at)} />
            <Row label="Listener" value={health?.http} />
            <Row label="Socket" value={d?.socket} />
            <Row label="rocket binary" value={d?.rocket_bin} />
            {d && !d.running && connection.state !== "offline" && (
              <Row label="Running" mono={false} value="Not running" />
            )}
            {stale && (
              <div role="status" className="selectable flex flex-wrap items-center gap-2 py-1.5 text-sm text-status-warn">
                <TriangleAlert className="size-4 shrink-0" aria-hidden />
                <span>{staleDaemonMessage(stale)}</span>
                <Button size="sm" variant="outline" onClick={() => setConfirmRestart(true)} disabled={restart.isPending}>
                  <RotateCcw /> Restart Daemon…
                </Button>
              </div>
            )}
          </Group>

          <Group title="Updates">
            <Row label="Version" value={env?.current_version} />
            <div className="grid grid-cols-[8.5rem_1fr] items-center gap-x-4 py-1.5">
              <MetaLabel>Automatic</MetaLabel>
              <label className="flex items-center gap-2 text-sm text-ink">
                <Switch
                  size="sm"
                  checked={updates.autoCheck}
                  onCheckedChange={(on) => updates.setAutoCheck(on)}
                  aria-label="Check for updates automatically"
                />
                Check for updates automatically
              </label>
            </div>
            <Row
              label="Last checked"
              mono={false}
              value={updates.lastChecked ? new Date(updates.lastChecked).toLocaleString() : "Never"}
            />
            {updates.available?.version && (
              <Row label="Available" value={`${updates.available.version} (you have ${updates.available.current_version})`} />
            )}
            <div className="flex flex-wrap items-center gap-2 pt-2">
              <Button variant="outline" size="sm" onClick={() => void checkNow()} disabled={updates.checking || updates.installing}>
                {updates.checking ? <Loader2 className="animate-spin" /> : <RefreshCw />} Check now
              </Button>
              {updates.available && (
                <Button size="sm" onClick={() => void installNow(updates.available!)} disabled={updates.installing}>
                  {updates.installing ? <Loader2 className="animate-spin" /> : <Download />} Install and Restart
                </Button>
              )}
            </div>
          </Group>

          <Group title="Location">
            <Row label="ROCKET_HOME" value={d?.home} />
            <Row label="daemon.json" value={d?.daemon_json} />
            <Row label="Database" value={d?.db} />
            <Row label="Logs" value={d?.logs} />
            {d?.warnings.map((w) => (
              <p key={w} role="alert" className="selectable flex items-start gap-2 py-1.5 text-sm text-status-warn">
                <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
                {w}
              </p>
            ))}
            <Button variant="outline" size="sm" className="mt-2" disabled={!d} onClick={() => d && reveal(d.home)}>
              <FolderOpen /> Reveal in Finder
            </Button>
          </Group>

          <Group title="Maintenance">
            <div className="flex flex-wrap items-center gap-2 py-1">
              <Button variant="outline" size="sm" onClick={reconnect} disabled={reconnecting}>
                {reconnecting ? <Loader2 className="animate-spin" /> : <PlugZap />} Reconnect
              </Button>
              <Button size="sm" onClick={() => setConfirmRestart(true)} disabled={restart.isPending}>
                {restart.isPending ? <Loader2 className="animate-spin" /> : <RotateCcw />} Restart Daemon…
              </Button>
              <span className="flex-1" />
              <Button variant="outline" size="sm" onClick={gc.collect} disabled={connection.state !== "online" || gc.pending}>
                {gc.pending ? <Loader2 className="animate-spin" /> : <Trash2 />} Collect Garbage
              </Button>
            </div>
            <p className="pt-1.5 text-xs text-secondary-ink">
              Restarting rocketd keeps running services: the new daemon adopts them on start.
            </p>
          </Group>
        </div>
      </div>

      <AlertDialog open={confirmRestart} onOpenChange={setConfirmRestart}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Restart rocketd?</AlertDialogTitle>
            <AlertDialogDescription>The app reconnects automatically.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmRestart(false);
                restart.mutate();
              }}
            >
              Restart
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
