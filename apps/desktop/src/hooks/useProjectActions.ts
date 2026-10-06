import { useIsMutating, useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router";
import { toast } from "sonner";
import type { JobRequest } from "@/lib/bindings";
import { queryKeys } from "@/lib/cache";
import { startJobOrConfirm } from "@/lib/deploy";
import { describeError, failureSummary } from "@/lib/errors";
import { rocket } from "@/lib/rocket";

export interface ActionTarget {
  /** Services to act on; undefined means the whole project. */
  services?: string[];
  env?: string;
}

/** Up / restart / down / pipeline for one project, with toasts and cache refresh. */
export function useProjectActions(project: string) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const refresh = () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: queryKeys.projectSummary(project) }),
      qc.invalidateQueries({ queryKey: queryKeys.runs }),
      qc.invalidateQueries({ queryKey: queryKeys.ports }),
    ]);
  const onError = (e: unknown) => {
    const { title, detail } = describeError(e);
    toast.error(title, { description: detail });
  };
  const key = ["action", project] as const;

  const up = useMutation({
    mutationKey: key,
    mutationFn: (t: ActionTarget) =>
      rocket.up({ project, services: t.services, env: t.env }),
    onSuccess: (r) => {
      const failures = failureSummary(r);
      if (failures) toast.error("Some services failed to start", { description: failures });
    },
    onError,
    onSettled: refresh,
  });
  const restart = useMutation({
    mutationKey: key,
    mutationFn: (t: ActionTarget) =>
      rocket.restart({ project, services: t.services, env: t.env }),
    onSuccess: (r) => {
      const failures = failureSummary(r);
      if (failures) toast.error("Some services failed to restart", { description: failures });
    },
    onError,
    onSettled: refresh,
  });
  const down = useMutation({
    mutationKey: key,
    mutationFn: (t: Pick<ActionTarget, "services">) =>
      rocket.down({ project, services: t.services }),
    onSuccess: (r) => {
      if (r.errors?.length) toast.error("Stop finished with errors", { description: r.errors.join("\n") });
    },
    onError,
    onSettled: refresh,
  });
  const job = useMutation({
    mutationKey: ["job-start", project],
    mutationFn: (request: JobRequest) => startJobOrConfirm(request),
    onSuccess: (j) => {
      if (j) {
        toast.success(`Started ${j.name}`, {
          action: { label: "View jobs", onClick: () => void navigate("/jobs") },
        });
        void qc.invalidateQueries({ queryKey: queryKeys.jobs });
      }
    },
    onError,
  });

  const busy = useIsMutating({ mutationKey: key }) > 0;
  return { up, restart, down, job, busy };
}
