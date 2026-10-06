import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { describeError } from "@/lib/errors";
import { rocket } from "@/lib/rocket";

/** Stops everything one owner started, in every project, and cancels its jobs. */
export function useStopOwner() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (owner: string) => rocket.down({ everywhere: true, owner }),
    onSuccess: (r, owner) => {
      if (r.errors?.length) toast.error("Stop finished with errors", { description: r.errors.join("\n") });
      else toast.success(`Stopped everything started by ${owner}`);
    },
    onError: (e) => {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail });
    },
    onSettled: () => qc.invalidateQueries(),
  });
}
