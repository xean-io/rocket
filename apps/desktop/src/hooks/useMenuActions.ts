import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { MENU_EVENT, type MenuHandlers, routeMenuAction } from "@/lib/menu";

/**
 * Registers the handlers a component owns for the native menu (`rocket://menu`).
 * Every component passes only the actions it can perform, so an action is
 * handled once even with several listeners mounted.
 */
export function useMenuActions(handlers: MenuHandlers): void {
  const ref = useRef(handlers);
  useEffect(() => {
    ref.current = handlers;
  });
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<string>(MENU_EVENT, (e) => routeMenuAction(e.payload, ref.current)).then(
      (un) => (disposed ? un() : (unlisten = un)),
      () => {},
    );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
}
