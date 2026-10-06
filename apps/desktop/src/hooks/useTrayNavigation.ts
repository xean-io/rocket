import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useNavigate } from "react-router";
import { projectPath } from "@/components/shell/AppSidebar";
import { rocket } from "@/lib/rocket";

/**
 * Backend-driven navigation: the tray's "Show in Rocket" (`rocket://open-project`)
 * and, in debug builds, the `ROCKET_INITIAL_ROUTE` used for visual checks.
 */
export function useBackendNavigation(): void {
  const navigate = useNavigate();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<string>("rocket://open-project", (e) => void navigate(projectPath(e.payload))).then(
      (un) => (disposed ? un() : (unlisten = un)),
      () => {},
    );
    rocket.initialRoute().then(
      (route) => {
        if (route && !disposed) void navigate(route, { replace: true });
      },
      () => {},
    );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigate]);
}
