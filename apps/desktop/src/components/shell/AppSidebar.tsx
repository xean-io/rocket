import { useMutation, useQueryClient } from "@tanstack/react-query";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FolderOpen, MinusCircle, Network, ListChecks, Plus, RefreshCw, Rocket, Settings, Users } from "lucide-react";
import { useMemo, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { toast } from "sonner";
import { StatusDot } from "@/components/StatusGlyph";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogAction,
} from "@/components/ui/alert-dialog";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
} from "@/components/ui/sidebar";
import { useAddProject } from "@/hooks/useAddProject";
import { useRefresh } from "@/hooks/useConnectionActions";
import type { Run } from "@/lib/bindings";
import { queryKeys } from "@/lib/cache";
import { describeError } from "@/lib/errors";
import { useIsOnline, useProjectNames, useProjects, useRuns } from "@/lib/queries";
import { rocket } from "@/lib/rocket";
import { isActive, statusCounts, type Tone } from "@/lib/status";
import { cn } from "@/lib/utils";

export const projectPath = (name: string) => `/project/${encodeURIComponent(name)}`;

/** Sidebar dot for a project: failures first, then transitions, then running. */
export function projectTone(runs: readonly Run[]): { tone: Tone; label: string; pulses: boolean } {
  const c = statusCounts(runs);
  if (c.down > 0) return { tone: "bad", label: "Has failed services", pulses: false };
  if (c.changing > 0) return { tone: "warn", label: "Changing", pulses: true };
  if (c.running > 0) return { tone: "ok", label: "Running", pulses: false };
  return { tone: "muted", label: "Stopped", pulses: false };
}

const rowClass =
  "h-7 gap-2 px-2 text-[0.93rem] hover:bg-sidebar-accent/50 data-active:bg-sidebar-accent data-active:font-semibold";

function Kbd({ children }: { children: React.ReactNode }) {
  return <kbd className="ml-auto font-mono text-[0.68rem] tracking-wider text-secondary-ink">{children}</kbd>;
}

export function AppSidebar() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const online = useIsOnline();
  const { names, loading } = useProjectNames();
  const projects = useProjects();
  const runs = useRuns();
  const addProject = useAddProject();
  const { refresh, fetching } = useRefresh();
  const [removing, setRemoving] = useState<string | null>(null);

  const byProject = useMemo(() => {
    const map = new Map<string, Run[]>();
    for (const r of runs.data?.services ?? []) map.set(r.project, [...(map.get(r.project) ?? []), r]);
    return map;
  }, [runs.data]);
  const ownerCount = useMemo(
    () => new Set((runs.data?.services ?? []).filter((r) => isActive(r.state)).map((r) => r.owner || "user")).size,
    [runs.data],
  );
  const pathOf = (name: string) => projects.data?.projects.find((p) => p.name === name)?.path;

  const remove = useMutation({
    mutationFn: (name: string) => rocket.removeProject(name),
    onSuccess: async (_r, name) => {
      await Promise.all([
        qc.invalidateQueries({ queryKey: queryKeys.projects }),
        qc.invalidateQueries({ queryKey: queryKeys.runs }),
      ]);
      qc.removeQueries({ queryKey: queryKeys.projectSummary(name) });
      if (pathname === projectPath(name)) void navigate("/");
    },
    onError: (e) => {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail });
    },
    onSettled: () => setRemoving(null),
  });

  const reveal = (path: string) => {
    revealItemInDir(path).catch((e) => toast.error("Could not open Finder", { description: String(e) }));
  };

  return (
    <Sidebar collapsible="none" className="hairline-r">
      <SidebarHeader data-tauri-drag-region className="hairline-b h-11 flex-row items-center gap-1.5 p-0 pr-3 pl-[78px]">
        <Rocket data-tauri-drag-region className="size-3.5 text-violet" aria-hidden />
        <span data-tauri-drag-region className="text-sm font-semibold tracking-tight">
          Rocket
        </span>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel className="h-6 text-[0.68rem] font-normal">
            <span className="meta">Projects</span>
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {loading && online && (
                <>
                  <SidebarMenuSkeleton />
                  <SidebarMenuSkeleton />
                </>
              )}
              {!loading && names.length === 0 && (
                <li className="px-2 py-1.5 text-sm text-secondary-ink">No projects yet</li>
              )}
              {names.map((name) => {
                const rs = byProject.get(name) ?? [];
                const { tone, label, pulses } = projectTone(rs);
                const running = rs.filter((r) => isActive(r.state)).length;
                const to = projectPath(name);
                const path = pathOf(name);
                return (
                  <ContextMenu key={name}>
                    <ContextMenuTrigger render={<SidebarMenuItem />}>
                      <SidebarMenuButton
                        className={rowClass}
                        isActive={pathname === to}
                        render={<Link to={to} />}
                        title={path ?? name}
                      >
                        <StatusDot tone={tone} pulses={pulses} label={label} />
                        <span className="truncate">{name}</span>
                      </SidebarMenuButton>
                      {running > 0 && (
                        <SidebarMenuBadge className="font-mono text-[0.7rem] text-secondary-ink top-1.5">
                          {running}/{rs.length}
                        </SidebarMenuBadge>
                      )}
                    </ContextMenuTrigger>
                    <ContextMenuContent className="min-w-52">
                      {path && (
                        <ContextMenuItem onClick={() => reveal(path)}>
                          <FolderOpen /> Reveal in Finder
                        </ContextMenuItem>
                      )}
                      {path && <ContextMenuSeparator />}
                      <ContextMenuItem variant="destructive" onClick={() => setRemoving(name)}>
                        <MinusCircle /> Remove from Rocket…
                      </ContextMenuItem>
                    </ContextMenuContent>
                  </ContextMenu>
                );
              })}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarGroup className="pt-0">
          <SidebarGroupLabel className="h-6 text-[0.68rem] font-normal">
            <span className="meta">Daemon</span>
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {[
                { to: "/ports", label: "Ports", icon: Network },
                { to: "/jobs", label: "Jobs", icon: ListChecks },
                { to: "/owners", label: "Owners", icon: Users },
              ].map(({ to, label, icon: Icon }) => (
                <SidebarMenuItem key={to}>
                  <SidebarMenuButton className={rowClass} isActive={pathname === to} render={<Link to={to} />}>
                    <Icon className={cn(pathname === to ? "text-ink" : "text-violet")} />
                    <span>{label}</span>
                  </SidebarMenuButton>
                  {to === "/owners" && ownerCount > 0 && (
                    <SidebarMenuBadge className="font-mono text-[0.7rem] text-secondary-ink top-1.5">
                      {ownerCount}
                    </SidebarMenuBadge>
                  )}
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter className="hairline-t p-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton className={rowClass} onClick={() => void addProject()} disabled={!online} title="Register a directory that contains rocket.yaml">
              <Plus className="text-violet" />
              <span>Add project…</span>
              <Kbd>⌘O</Kbd>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton className={rowClass} onClick={refresh} title="Refresh projects and daemon state">
              <RefreshCw className={cn("text-violet", fetching && "animate-spin motion-reduce:animate-none")} />
              <span>Refresh</span>
              <Kbd>⇧⌘R</Kbd>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton className={rowClass} isActive={pathname === "/settings"} render={<Link to="/settings" />}>
              <Settings className={cn(pathname === "/settings" ? "text-ink" : "text-violet")} />
              <span>Settings</span>
              <Kbd>⌘,</Kbd>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>

      <AlertDialog open={removing !== null} onOpenChange={(o) => !o && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing} from Rocket?</AlertDialogTitle>
            <AlertDialogDescription>
              Only the registration is removed; files stay untouched. A running project must be stopped first.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => removing && remove.mutate(removing)}
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Sidebar>
  );
}
