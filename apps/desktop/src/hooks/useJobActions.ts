import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router";
import { toast } from "sonner";
import type { Job } from "@/lib/bindings";
import { applyJob, queryKeys } from "@/lib/cache";
import { startJobOrConfirm } from "@/lib/deploy";
import { describeError } from "@/lib/errors";
import { jobTitle, rerunRequest } from "@/lib/jobs";
import { useNav } from "@/lib/nav";
import { rocket } from "@/lib/rocket";

/** Cancel and Run Again for jobs in the Jobs page. */
export function useJobActions() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const onError = (e: unknown) => {
    const { title, detail } = describeError(e);
    toast.error(title, { description: detail });
  };

  const cancel = useMutation({
    mutationFn: (job: Job) => rocket.cancelJob(job.id),
    onSuccess: (job) => applyJob(qc, job),
    onError,
    onSettled: () => qc.invalidateQueries({ queryKey: queryKeys.jobs }),
  });

  const rerun = useMutation({
    mutationFn: async (job: Job) => {
      const request = rerunRequest(job);
      if (job.kind === "deploy") {
        // A deploy never reruns without an explicit yes in the dialog.
        useNav.getState().requestDeploy({
          project: job.project,
          env: request.env ?? job.name,
          request,
          rerun: true,
        });
        return null;
      }
      return startJobOrConfirm(request);
    },
    onSuccess: (job) => {
      if (!job) return;
      applyJob(qc, job);
      useNav.getState().selectJob(job.id);
      void navigate("/jobs");
      toast.success(`Started ${jobTitle(job)}`);
    },
    onError,
  });

  return { cancel, rerun };
}
