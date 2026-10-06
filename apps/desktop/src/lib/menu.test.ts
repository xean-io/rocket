import { describe, expect, it, vi } from "vitest";
import { MENU_ACTIONS, isMenuAction, routeMenuAction } from "./menu";

describe("routeMenuAction", () => {
  it("calls the matching handler and reports it handled", () => {
    const up = vi.fn();
    const ports = vi.fn();
    expect(routeMenuAction("services.up", { "services.up": up, "view.ports": ports })).toBe(true);
    expect(up).toHaveBeenCalledOnce();
    expect(ports).not.toHaveBeenCalled();
  });

  it("leaves actions this component does not own alone, so two listeners never double-fire", () => {
    const a = vi.fn();
    const b = vi.fn();
    const ownA = { "view.jobs": a } as const;
    const ownB = { "services.stop": b } as const;
    routeMenuAction("services.stop", ownA);
    routeMenuAction("services.stop", ownB);
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledOnce();
    expect(routeMenuAction("services.stop", ownA)).toBe(false);
  });

  it("ignores unknown ids and non-string payloads", () => {
    const h = vi.fn();
    expect(routeMenuAction("copy", { "services.up": h })).toBe(false);
    expect(routeMenuAction(42, { "services.up": h })).toBe(false);
    expect(h).not.toHaveBeenCalled();
  });
});

describe("action ids", () => {
  it("lists exactly the ids the native menu emits", () => {
    expect([...MENU_ACTIONS].sort()).toEqual(
      [
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
      ].sort(),
    );
    expect(isMenuAction("view.ports")).toBe(true);
    expect(isMenuAction("nope")).toBe(false);
  });
});
