import { useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback } from "react";
import { useNavigate } from "react-router";
import { toast } from "sonner";
import { queryKeys } from "@/lib/cache";
import { describeError } from "@/lib/errors";
import { rocket } from "@/lib/rocket";

/** Folder picker -> `addProject`, then opens the new project. */
export function useAddProject() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return useCallback(async () => {
    const dir = await open({
      directory: true,
      multiple: false,
      title: "Add project",
    });
    if (typeof dir !== "string") return;
    try {
      const project = await rocket.addProject(dir);
      await Promise.all([
        qc.invalidateQueries({ queryKey: queryKeys.projects }),
        qc.invalidateQueries({ queryKey: queryKeys.runs }),
      ]);
      void navigate(`/project/${encodeURIComponent(project.name)}`);
      toast.success(`Added ${project.name}`);
    } catch (e) {
      const { title, detail } = describeError(e);
      toast.error(title, { description: detail ?? "Pick a folder that contains rocket.yaml." });
    }
  }, [qc, navigate]);
}
