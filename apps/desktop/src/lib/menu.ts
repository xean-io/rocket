// Native menu actions. The Rust side (src-tauri/src/menu.rs) emits
// `rocket://menu` with one of these ids; components register the handlers they
// own, and the native accelerators replace the old webview key handling.

export const MENU_EVENT = "rocket://menu";

export const MENU_ACTIONS = [
  "app.add_project",
  "app.settings",
  "services.up",
  "services.restart",
  "services.stop",
  "services.logs",
  "services.refresh",
  "services.gc",
  "view.projects",
  "view.ports",
  "view.jobs",
  "view.owners",
] as const;

export type MenuAction = (typeof MENU_ACTIONS)[number];

export const isMenuAction = (v: unknown): v is MenuAction =>
  typeof v === "string" && (MENU_ACTIONS as readonly string[]).includes(v);

export type MenuHandlers = Partial<Record<MenuAction, () => void>>;

/** Runs the handler registered for `payload`; `true` when this map owned it. */
export function routeMenuAction(payload: unknown, handlers: MenuHandlers): boolean {
  if (!isMenuAction(payload)) return false;
  const handler = handlers[payload];
  if (!handler) return false;
  handler();
  return true;
}
