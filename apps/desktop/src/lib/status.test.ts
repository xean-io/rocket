import { describe, expect, it } from "vitest";
import type { Run } from "./bindings";
import { isActive, statusCounts, statusStyle } from "./status";

const run = (state: Run["state"], service: string = state, health: Run["health"] = "unknown"): Run => ({
  project: "app",
  service,
  env: "dev",
  kind: "run",
  state,
  health,
});

describe("statusCounts", () => {
  it("counts running, changing and down services", () => {
    const runs = [
      run("running", "a"),
      run("running", "b", "unhealthy"),
      run("starting"),
      run("stopping"),
      run("failed"),
      run("dead"),
      run("exited"),
      run("stopped"),
    ];
    expect(statusCounts(runs)).toEqual({ running: 2, changing: 2, down: 3 });
  });

  it("is all zero for no runs", () => {
    expect(statusCounts([])).toEqual({ running: 0, changing: 0, down: 0 });
  });
});

describe("statusStyle", () => {
  it("maps semantic states", () => {
    expect(statusStyle("running")).toMatchObject({ tone: "ok", label: "Running", pulses: false });
    expect(statusStyle("running", "unhealthy")).toMatchObject({ tone: "warn", label: "Unhealthy" });
    expect(statusStyle("starting")).toMatchObject({ tone: "warn", pulses: true });
    expect(statusStyle("stopped")).toMatchObject({ tone: "muted", label: "Stopped" });
    expect(statusStyle("failed")).toMatchObject({ tone: "bad", label: "Failed" });
    expect(statusStyle("dead")).toMatchObject({ tone: "bad", label: "Dead" });
  });
});

describe("isActive", () => {
  it("is true while starting, running or stopping", () => {
    expect(["starting", "running", "stopping"].every((s) => isActive(s as Run["state"]))).toBe(true);
    expect(["stopped", "exited", "failed", "dead"].some((s) => isActive(s as Run["state"]))).toBe(false);
  });
});
