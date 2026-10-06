import { describe, expect, it } from "vitest";
import type { ProjectInfo, Run } from "./bindings";
import { knownEnvs, reconcileEnvSelections, validatedEnv } from "./envs";

const info = (over: Partial<ProjectInfo> = {}): ProjectInfo => ({
  name: "app",
  root: "/p/app",
  default_env: "dev",
  ...over,
});

const run = (env: string): Run => ({
  project: "app",
  service: "web",
  env,
  kind: "run",
  state: "running",
  health: "unknown",
});

describe("knownEnvs", () => {
  it("treats declared envs as authoritative, sorted", () => {
    expect(knownEnvs(info({ envs: ["prod", "dev"] }), [run("stale")])).toEqual(["dev", "prod"]);
  });

  it("keeps an explicitly empty list empty", () => {
    expect(knownEnvs(info({ envs: [] }), [run("dev")])).toEqual([]);
  });

  it("falls back to default + run envs for older daemons (envs absent or null)", () => {
    expect(knownEnvs(info(), [run("qa")])).toEqual(["dev", "qa"]);
    expect(knownEnvs(info({ envs: null }), [])).toEqual(["dev"]);
  });

  it("is empty without project info", () => {
    expect(knownEnvs(undefined, [])).toEqual([]);
  });
});

describe("validatedEnv", () => {
  it("returns the selection only when it is known", () => {
    expect(validatedEnv("prod", ["dev", "prod"])).toBe("prod");
    expect(validatedEnv("gone", ["dev", "prod"])).toBeUndefined();
    expect(validatedEnv(undefined, ["dev"])).toBeUndefined();
  });
});

describe("reconcileEnvSelections", () => {
  it("drops selections whose env disappeared and keeps the rest", () => {
    const next = reconcileEnvSelections(
      { a: "prod", b: "qa", c: "dev" },
      { a: ["dev", "prod"], b: ["dev"], c: ["dev"] },
    );
    expect(next).toEqual({ a: "prod", c: "dev" });
  });

  it("returns the same object when nothing changed", () => {
    const sel = { a: "prod" };
    expect(reconcileEnvSelections(sel, { a: ["prod"] })).toBe(sel);
  });

  it("keeps selections of projects whose options are not loaded yet", () => {
    const sel = { a: "prod" };
    expect(reconcileEnvSelections(sel, {})).toBe(sel);
  });
});
