import { Boxes } from "lucide-react";
import { useMemo, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { ProjectHeader } from "@/components/project/ProjectHeader";
import { ProjectToolbar } from "@/components/project/ProjectToolbar";
import { ServiceInspector } from "@/components/project/ServiceInspector";
import { ServicesTable } from "@/components/project/ServicesTable";
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
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { useProjectActions } from "@/hooks/useProjectActions";
import { useReconcileEnv } from "@/hooks/useReconcileEnvs";
import { useShortcuts } from "@/hooks/useShortcuts";
import { knownEnvs, validatedEnv } from "@/lib/envs";
import { useNav } from "@/lib/nav";
import { sortedServices, useIsOnline, useProjects, useProjectSummary } from "@/lib/queries";

/** One project: header, toolbar, services table and the logs inspector. */
export function ProjectView({ project }: { project: string }) {
  const online = useIsOnline();
  const summary = useProjectSummary(project);
  const projects = useProjects();
  const actions = useProjectActions(project);
  const [confirmStop, setConfirmStop] = useState(false);

  const selection = useNav((s) => s.selectedService);
  const selectService = useNav((s) => s.selectService);
  const inspectorOpen = useNav((s) => s.inspectorOpen);
  const setInspectorOpen = useNav((s) => s.setInspectorOpen);
  const toggleInspector = useNav((s) => s.toggleInspector);
  const requestDeploy = useNav((s) => s.requestDeploy);
  const setEnv = useNav((s) => s.setEnv);
  const selectedEnv = useNav((s) => s.envs[project]);

  const info = summary.data?.project;
  const runs = useMemo(() => sortedServices(summary.data?.services ?? []), [summary.data]);
  const conflicts = summary.data?.conflicts ?? [];
  const envs = useMemo(() => (info ? knownEnvs(info, runs) : undefined), [info, runs]);
  useReconcileEnv(project, envs);
  const env = validatedEnv(selectedEnv, envs ?? []);

  const selected = selection?.project === project ? (runs.find((r) => r.service === selection.service) ?? null) : null;
  const services = selected ? [selected.service] : undefined;
  const path = projects.data?.projects.find((p) => p.name === project)?.path ?? info?.root;

  const up = (svc?: string[]) => actions.up.mutate({ services: svc ?? services, env });
  const restart = (svc?: string[]) => actions.restart.mutate({ services: svc ?? services, env });
  const down = (svc?: string[]) => {
    const target = svc ?? services;
    if (target) actions.down.mutate({ services: target });
    else setConfirmStop(true);
  };
  const showLogs = (service: string) => {
    selectService({ project, service });
    setInspectorOpen(true);
  };

  useShortcuts([
    { key: "u", shift: true, handler: () => up() },
    { key: "r", handler: () => restart() },
    { key: ".", handler: () => down() },
    { key: "l", handler: toggleInspector },
  ]);

  const hasServices = runs.length > 0;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <TopBar>
        <ProjectToolbar
          selected={selected}
          onClearSelection={() => selectService(null)}
          envs={envs ?? []}
          defaultEnv={info?.default_env}
          env={env}
          onEnvChange={(e) => setEnv(project, e)}
          pipelines={info?.pipelines ?? []}
          deployEnvs={info?.deploy_envs ?? []}
          busy={actions.busy}
          online={online}
          inspectorOpen={inspectorOpen}
          onUp={() => up()}
          onRestart={() => restart()}
          onDown={() => down()}
          onRunPipeline={(name) => actions.job.mutate({ project, kind: "pipeline", name, env })}
          onDeploy={(target) => requestDeploy({ project, env: target })}
          onToggleInspector={toggleInspector}
        />
      </TopBar>

      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel id="services" minSize={360}>
          <div className="flex h-full min-h-0 flex-col">
            <ProjectHeader name={project} path={path} runs={runs} conflicts={conflicts} busy={actions.busy} />
            <div className="hairline-t flex min-h-0 flex-1 flex-col">
              {!summary.isPending && !hasServices ? (
                <EmptyState
                  icon={Boxes}
                  title="No services"
                  description="rocket.yaml declares no services, or the project is not loaded yet."
                />
              ) : (
                <ServicesTable
                  runs={runs}
                  loading={summary.isPending}
                  selected={selected?.service ?? null}
                  busy={actions.busy}
                  onSelect={(service) => selectService(service ? { project, service } : null)}
                  actions={{
                    onUp: (s) => up([s]),
                    onRestart: (s) => restart([s]),
                    onStop: (s) => down([s]),
                    onShowLogs: showLogs,
                  }}
                />
              )}
            </div>
          </div>
        </ResizablePanel>
        {inspectorOpen && (
          <>
            <ResizableHandle />
            <ResizablePanel id="inspector" defaultSize={380} minSize={300} maxSize={620} groupResizeBehavior="preserve-pixel-size">
              <ServiceInspector key={selected?.service ?? "none"} run={selected} />
            </ResizablePanel>
          </>
        )}
      </ResizablePanelGroup>

      <AlertDialog open={confirmStop} onOpenChange={setConfirmStop}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Stop every service in {project}?</AlertDialogTitle>
            <AlertDialogDescription>
              Compose projects are taken down too. Running jobs of this project are canceled.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmStop(false);
                actions.down.mutate({});
              }}
            >
              Stop all
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
