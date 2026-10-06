import { useEffect } from "react";
import { reconcileEnvSelections } from "@/lib/envs";
import { useNav } from "@/lib/nav";

/** Prunes env selections whose env is no longer declared (resets to default). */
export function useReconcileEnv(project: string, known: readonly string[] | undefined) {
  useEffect(() => {
    if (!known) return;
    const { envs, replaceEnvs } = useNav.getState();
    const next = reconcileEnvSelections(envs, { [project]: known });
    if (next !== envs) replaceEnvs(next);
  }, [project, known]);
}
