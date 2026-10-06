import { Plus, Rocket } from "lucide-react";
import { useEffect } from "react";
import { toast } from "sonner";
import { Navigate, Route, Routes, useLocation, useNavigate, useParams } from "react-router";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { DeployDialog } from "@/components/DeployDialog";
import { EmptyState } from "@/components/EmptyState";
import { OwnersPage } from "@/components/pages/OwnersPage";
import { JobsPage } from "@/components/pages/JobsPage";
import { PortsPage } from "@/components/pages/PortsPage";
import { SettingsPage } from "@/components/pages/SettingsPage";
import { ProjectView } from "@/components/project/ProjectView";
import { AppSidebar, projectPath } from "@/components/shell/AppSidebar";
import { ConnectionStateView, ConnectionStrip } from "@/components/shell/ConnectionViews";
import { TopBar } from "@/components/shell/TopBar";
import { useAddProject } from "@/hooks/useAddProject";
import { useRefresh } from "@/hooks/useConnectionActions";
import { useRocketEvents } from "@/hooks/useRocketEvents";
import { useGc } from "@/hooks/useGc";
import { useStaleDaemon, useUpdates } from "@/hooks/useUpdates";
import { useMenuActions } from "@/hooks/useMenuActions";
import { useBackendNavigation } from "@/hooks/useTrayNavigation";
import { useIsOnline, useProjectNames, useProjects } from "@/lib/queries";
import { staleDaemonMessage } from "@/lib/updates";

function IndexRoute() {
  const { names, loading } = useProjectNames();
  const online = useIsOnline();
  const addProject = useAddProject();
  if (loading && online) {
    return (
      <div className="flex h-full flex-col">
        <TopBar />
        <div className="space-y-3 p-6">
          <Skeleton className="h-7 w-48" />
          <Skeleton className="h-4 w-72" />
        </div>
      </div>
    );
  }
  if (names.length > 0) return <Navigate to={projectPath(names[0])} replace />;
  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <EmptyState
        icon={Rocket}
        title="No project selected"
        description="Add a directory with rocket.yaml, or run `rocket up` in one."
      >
        <Button onClick={() => void addProject()} disabled={!online}>
          <Plus /> Add project…
        </Button>
      </EmptyState>
    </div>
  );
}

function ProjectRoute() {
  const { name } = useParams();
  const { names, loading } = useProjectNames();
  if (!name) return <Navigate to="/" replace />;
  if (!loading && !names.includes(name)) return <Navigate to="/" replace />;
  return <ProjectView key={name} project={name} />;
}

/** Native-menu actions that are not tied to one page: navigation, refresh, GC. */
function GlobalMenuActions() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { names } = useProjectNames();
  const addProject = useAddProject();
  const { refresh } = useRefresh();
  const gc = useGc();
  useMenuActions({
    "view.projects": () => {
      if (!pathname.startsWith("/project/") && names[0]) void navigate(projectPath(names[0]));
      else if (!names[0]) void navigate("/");
    },
    "view.ports": () => void navigate("/ports"),
    "view.jobs": () => void navigate("/jobs"),
    "view.owners": () => void navigate("/owners"),
    "app.settings": () => void navigate("/settings"),
    "services.refresh": refresh,
    "services.gc": gc.collect,
    "app.add_project": () => void addProject(),
  });
  useBackendNavigation();
  return null;
}

/** After an update the old daemon keeps running; point at Settings, never restart it. */
function StaleDaemonToast() {
  const navigate = useNavigate();
  const stale = useStaleDaemon().data;
  const running = stale?.running;
  const bundled = stale?.bundled;
  useEffect(() => {
    if (!running || !bundled) return;
    toast.info(staleDaemonMessage({ running, bundled }), {
      id: "daemon-stale",
      duration: 15_000,
      action: { label: "Settings", onClick: () => void navigate("/settings") },
    });
  }, [running, bundled, navigate]);
  return null;
}

function Content() {
  const online = useIsOnline();
  const projects = useProjects();
  // Nothing cached and no connection: the whole pane explains the state.
  if (!online && !projects.data) {
    return (
      <div className="flex h-full flex-col">
        <TopBar />
        <ConnectionStateView />
      </div>
    );
  }
  return (
    <Routes>
      <Route index element={<IndexRoute />} />
      <Route path="project/:name" element={<ProjectRoute />} />
      <Route path="ports" element={<PortsPage />} />
      <Route path="jobs" element={<JobsPage />} />
      <Route path="owners" element={<OwnersPage />} />
      <Route path="settings" element={<SettingsPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

/** Window chrome: sidebar, routed main pane, connection strip, global dialogs. */
export function AppShell() {
  useRocketEvents();
  useUpdates();
  return (
    <SidebarProvider className="h-svh min-h-0 overflow-hidden" style={{ "--sidebar-width": "15rem" } as React.CSSProperties}>
      <GlobalMenuActions />
      <StaleDaemonToast />
      <AppSidebar />
      <SidebarInset className="xean-backdrop min-h-0 min-w-0 overflow-hidden">
        <div className="min-h-0 flex-1">
          <Content />
        </div>
        <ConnectionStrip />
      </SidebarInset>
      <DeployDialog />
      <Toaster position="bottom-right" />
    </SidebarProvider>
  );
}
