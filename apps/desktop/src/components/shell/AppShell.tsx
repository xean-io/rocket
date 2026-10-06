import { ListChecks, Network, Plus, Rocket, Settings, Users } from "lucide-react";
import { Navigate, Route, Routes, useLocation, useNavigate, useParams } from "react-router";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { DeployDialog } from "@/components/DeployDialog";
import { EmptyState } from "@/components/EmptyState";
import { ProjectView } from "@/components/project/ProjectView";
import { AppSidebar, projectPath } from "@/components/shell/AppSidebar";
import { ConnectionStateView, ConnectionStrip } from "@/components/shell/ConnectionViews";
import { StubPage } from "@/components/shell/StubPage";
import { TopBar } from "@/components/shell/TopBar";
import { useAddProject } from "@/hooks/useAddProject";
import { useRefresh } from "@/hooks/useConnectionActions";
import { useRocketEvents } from "@/hooks/useRocketEvents";
import { useShortcuts } from "@/hooks/useShortcuts";
import { useIsOnline, useProjectNames, useProjects } from "@/lib/queries";

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

function GlobalShortcuts() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { names } = useProjectNames();
  const addProject = useAddProject();
  const { refresh } = useRefresh();
  useShortcuts([
    {
      key: "1",
      handler: () => {
        if (!pathname.startsWith("/project/") && names[0]) void navigate(projectPath(names[0]));
        else if (!names[0]) void navigate("/");
      },
    },
    { key: "2", handler: () => void navigate("/ports") },
    { key: "3", handler: () => void navigate("/jobs") },
    { key: "4", handler: () => void navigate("/owners") },
    { key: ",", handler: () => void navigate("/settings") },
    { key: "r", shift: true, handler: refresh },
    { key: "o", handler: () => void addProject() },
  ]);
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
      <Route
        path="ports"
        element={
          <StubPage
            title="Ports"
            icon={Network}
            description="Every port Rocket has leased, the service that holds it and any conflicts will be listed here."
          />
        }
      />
      <Route
        path="jobs"
        element={
          <StubPage
            title="Jobs"
            icon={ListChecks}
            description="Pipeline, setup and deploy runs with their live logs will be listed here."
          />
        }
      />
      <Route
        path="owners"
        element={
          <StubPage
            title="Owners"
            icon={Users}
            description="Running services grouped by who started them, with a way to stop each group, will be listed here."
          />
        }
      />
      <Route
        path="settings"
        element={
          <StubPage
            title="Settings"
            icon={Settings}
            description="Daemon location, restart and cleanup controls will be available here."
          />
        }
      />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

/** Window chrome: sidebar, routed main pane, connection strip, global dialogs. */
export function AppShell() {
  useRocketEvents();
  return (
    <SidebarProvider className="h-svh min-h-0 overflow-hidden" style={{ "--sidebar-width": "15rem" } as React.CSSProperties}>
      <GlobalShortcuts />
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
