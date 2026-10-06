import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Send } from "lucide-react";
import { useNavigate } from "react-router";
import { toast } from "sonner";
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
import { applyJob, queryKeys } from "@/lib/cache";
import { confirmDeploy } from "@/lib/deploy";
import { describeError } from "@/lib/errors";
import { useNav } from "@/lib/nav";

/**
 * The one confirmation for every deploy: opened from the Deploy menu and when
 * the daemon answers 428. Cancel sends nothing; Deploy resends with `yes: true`.
 */
export function DeployDialog() {
  const pending = useNav((s) => s.pendingDeploy);
  const clear = useNav((s) => s.clearDeploy);
  const qc = useQueryClient();
  const navigate = useNavigate();

  const deploy = useMutation({
    mutationFn: confirmDeploy,
    onSuccess: (job) => {
      const rerun = useNav.getState().pendingDeploy?.rerun;
      clear();
      applyJob(qc, job);
      void qc.invalidateQueries({ queryKey: queryKeys.jobs });
      if (rerun) useNav.getState().selectJob(job.id);
      toast.success(`Deploying to ${job.env || job.name}`, {
        action: { label: "View jobs", onClick: () => void navigate("/jobs") },
      });
    },
    onError: (e) => {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail });
      clear();
    },
  });

  return (
    <AlertDialog open={pending !== null} onOpenChange={(open) => !open && !deploy.isPending && clear()}>
      <AlertDialogContent>
        {pending && (
          <>
            <AlertDialogHeader>
              <AlertDialogTitle>Deploy to {pending.env}?</AlertDialogTitle>
              <AlertDialogDescription>
                {pending.rerun
                  ? `Run ${pending.project}'s deployment to ${pending.env} again.`
                  : `Run ${pending.project}'s configured deployment to ${pending.env}.`}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel disabled={deploy.isPending}>Cancel</AlertDialogCancel>
              <AlertDialogAction autoFocus disabled={deploy.isPending} onClick={() => deploy.mutate(pending)}>
                <Send /> Deploy
              </AlertDialogAction>
            </AlertDialogFooter>
          </>
        )}
      </AlertDialogContent>
    </AlertDialog>
  );
}
