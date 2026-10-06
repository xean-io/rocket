import { useQuery } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useRocketEvents } from "@/hooks/useRocketEvents";
import type { ConnectionStatus } from "@/lib/bindings";
import { queryKeys } from "@/lib/cache";
import { useLive } from "@/lib/live";
import { rocket } from "@/lib/rocket";

function ConnectionBadge({ status }: { status: ConnectionStatus }) {
  switch (status.state) {
    case "online":
      return <Badge>Online</Badge>;
    case "connecting":
      return <Badge variant="secondary">Connecting…</Badge>;
    case "offline":
      return (
        <Badge variant="destructive" title={status.reason}>
          Offline
        </Badge>
      );
  }
}

// Proof of life for the Tauri bridge; the real shell arrives in T2.
export default function App() {
  useRocketEvents();
  const connection = useLive((s) => s.connection);
  const online = connection.state === "online";
  const health = useQuery({ queryKey: queryKeys.health, queryFn: rocket.health, enabled: online });
  const projects = useQuery({
    queryKey: queryKeys.projects,
    queryFn: rocket.projects,
    enabled: online,
  });

  return (
    <main className="min-h-screen bg-background p-6 pt-10 text-foreground">
      <div data-tauri-drag-region className="fixed inset-x-0 top-0 h-8" />
      <Card className="mx-auto max-w-xl">
        <CardHeader>
          <CardTitle className="flex items-center justify-between">
            Rocket
            <ConnectionBadge status={connection} />
          </CardTitle>
          <CardDescription>
            {connection.state === "offline"
              ? connection.reason
              : health.data
                ? `rocketd ${health.data.version} (pid ${health.data.pid})`
                : "Waiting for the daemon"}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          <h2 className="text-sm font-medium">Projects</h2>
          {projects.isPending && online ? (
            <Skeleton className="h-5 w-full" />
          ) : projects.data?.projects.length ? (
            <ul className="space-y-1 text-sm">
              {projects.data.projects.map((p) => (
                <li key={p.name} className="flex justify-between gap-4">
                  <span className="font-medium">{p.name}</span>
                  <span className="truncate text-muted-foreground">{p.path}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-sm text-muted-foreground">No projects registered.</p>
          )}
        </CardContent>
      </Card>
    </main>
  );
}
