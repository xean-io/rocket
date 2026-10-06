import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { describeError } from "@/lib/errors";
import { gcSummary } from "@/lib/gc";
import { rocket } from "@/lib/rocket";

/** Collect Garbage with a one-line toast of what was cleaned. */
export function useGc() {
  const qc = useQueryClient();
  const m = useMutation({
    mutationFn: rocket.gc,
    onSuccess: (result) => toast.success("Garbage collected", { description: gcSummary(result) }),
    onError: (e) => {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail });
    },
    onSettled: () => qc.invalidateQueries(),
  });
  return { collect: () => m.mutate(), pending: m.isPending };
}
