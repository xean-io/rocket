// Window-level UI state shared by the sidebar, shortcuts and dialogs.
import { create } from "zustand";
import type { JobRequest } from "./bindings";

export interface ServiceKey {
  project: string;
  service: string;
}

/** A deploy waiting for the human's explicit yes. */
export interface DeployIntent {
  project: string;
  env: string;
  /** The request to resend with `yes: true`; defaults to a plain deploy. */
  request?: JobRequest;
  /** Running a finished deploy again (changes the dialog copy only). */
  rerun?: boolean;
}

interface NavState {
  selectedService: ServiceKey | null;
  /** Job shown in the Jobs inspector. */
  selectedJob: string | null;
  inspectorOpen: boolean;
  /** Env picked in the project toolbar, per project (absent = project default). */
  envs: Record<string, string>;
  pendingDeploy: DeployIntent | null;
  selectService: (key: ServiceKey | null) => void;
  selectJob: (id: string | null) => void;
  setInspectorOpen: (open: boolean) => void;
  toggleInspector: () => void;
  setEnv: (project: string, env: string | undefined) => void;
  replaceEnvs: (envs: Record<string, string>) => void;
  requestDeploy: (intent: DeployIntent) => void;
  clearDeploy: () => void;
}

export const useNav = create<NavState>((set) => ({
  selectedService: null,
  selectedJob: null,
  inspectorOpen: false,
  envs: {},
  pendingDeploy: null,
  selectService: (selectedService) => set({ selectedService }),
  selectJob: (selectedJob) => set({ selectedJob }),
  setInspectorOpen: (inspectorOpen) => set({ inspectorOpen }),
  toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
  setEnv: (project, env) =>
    set((s) => {
      const envs = { ...s.envs };
      if (env === undefined) delete envs[project];
      else envs[project] = env;
      return { envs };
    }),
  replaceEnvs: (envs) => set({ envs }),
  requestDeploy: (pendingDeploy) => set({ pendingDeploy }),
  clearDeploy: () => set({ pendingDeploy: null }),
}));
