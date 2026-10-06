// Wires the update controller to Tauri, the toast area and a small store the
// Settings page reads. Pure decisions live in `updates.ts`.
import { toast } from "sonner";
import { create } from "zustand";
import type { UpdateStatus } from "./bindings";
import { describeError } from "./errors";
import { RocketError, installUpdate, rocket } from "./rocket";
import {
  createUpdateController,
  progressLabel,
  readAutoCheck,
  readLastChecked,
  writeAutoCheck,
  writeLastChecked,
} from "./updates";

const TOAST_ID = "rocket-update";

interface UpdateStore {
  autoCheck: boolean;
  checking: boolean;
  installing: boolean;
  /** The update found by the last check, if any. */
  available: UpdateStatus | null;
  lastChecked: number | null;
  setAutoCheck: (on: boolean) => void;
}

export const useUpdateStore = create<UpdateStore>((set) => ({
  autoCheck: readAutoCheck(),
  checking: false,
  installing: false,
  available: null,
  lastChecked: readLastChecked(),
  setAutoCheck: (on) => {
    writeAutoCheck(on);
    set({ autoCheck: on });
  },
}));

const set = useUpdateStore.setState;

/** Downloads, verifies and installs; the app relaunches itself when done. */
export async function installNow(status: UpdateStatus): Promise<void> {
  if (useUpdateStore.getState().installing) return;
  const version = status.version ?? "update";
  let received = 0;
  let total: number | null = null;
  set({ installing: true });
  toast.loading(`Downloading Rocket ${version}`, { id: TOAST_ID, description: "Starting…", duration: Infinity });
  try {
    await installUpdate((e) => {
      if (e.event === "started") total = e.content_length;
      else if (e.event === "progress") received += e.chunk;
      else {
        toast.loading(`Installing Rocket ${version}`, { id: TOAST_ID, description: "Rocket restarts when it is done." });
        return;
      }
      toast.loading(`Downloading Rocket ${version}`, {
        id: TOAST_ID,
        description: progressLabel(received, total),
        duration: Infinity,
      });
    });
  } catch (e) {
    set({ installing: false });
    const { title, detail } = describeError(e);
    toast.error("Update failed", { id: TOAST_ID, description: detail ?? title, duration: Infinity, dismissible: true });
  }
}

function announce(status: UpdateStatus) {
  const version = status.version ?? "";
  set({ available: status });
  toast(`Rocket ${version} is available`, {
    id: TOAST_ID,
    description: status.notes?.trim() || `You have ${status.current_version}.`,
    duration: Infinity,
    action: {
      label: "Install and Restart",
      // Keep the toast: it turns into the progress indicator.
      onClick: (ev) => {
        ev.preventDefault();
        void installNow(status);
      },
    },
    cancel: { label: "Later", onClick: () => controller.later(version) },
    onDismiss: () => controller.later(version),
  });
}

const controller = createUpdateController({
  check: async () => {
    set({ checking: true });
    try {
      const status = await rocket.checkUpdate();
      set({ available: status.available ? status : null });
      return status;
    } finally {
      set({ checking: false });
    }
  },
  ui: {
    announce,
    upToDate: (s) => toast.success("Rocket is up to date", { description: `Version ${s.current_version}` }),
    failed: (e) => {
      // A check that overlaps another operation is not a failure worth a toast.
      if (e instanceof RocketError && e.code === "busy") return;
      const { title, detail } = describeError(e);
      toast.error("Could not check for updates", { description: detail ?? title });
    },
  },
  onChecked: (at) => {
    writeLastChecked(at);
    set({ lastChecked: at });
  },
});

/** Settings, menu and tray: always answers the user. */
export const checkNow = () => controller.check({ manual: true });

/** Launch and periodic checks: quiet unless something new is available. */
export const checkQuietly = () => controller.check({ manual: false });
