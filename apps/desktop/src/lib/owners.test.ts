import { describe, expect, it } from "vitest";
import type { Job, Run } from "./bindings";
import { ownerGroups, ownerCount } from "./owners";

const run = (service: string, over: Partial<Run> = {}): Run => ({
  project: "app",
  service,
  env: "dev",
  kind: "run",
  state: "running",
  health: "unknown",
  ...over,
});
const job = (id: string, over: Partial<Job> = {}): Job => ({
  id,
  project: "app",
  name: "build",
  kind: "pipeline",
  owner: "user",
  steps: [],
  status: "running",
  started_at: "2026-01-01T00:00:00Z",
  duration_ms: 0,
  ...over,
});

describe("ownerGroups", () => {
  it("groups active runs by owner, empty owner meaning user, sorted by owner", () => {
    const groups = ownerGroups(
      [
        run("web", { owner: "agent:claude-1" }),
        run("api"),
        run("db", { owner: "user" }),
        run("old", { state: "stopped" }),
        run("boot", { state: "starting", owner: "agent:claude-1" }),
      ],
      [],
    );
    expect(groups.map((g) => g.owner)).toEqual(["agent:claude-1", "user"]);
    expect(groups[0]?.runs.map((r) => r.service)).toEqual(["boot", "web"]);
    expect(groups[1]?.runs.map((r) => r.service)).toEqual(["api", "db"]);
    expect(groups[0]?.isAgent).toBe(true);
    expect(groups[1]?.isAgent).toBe(false);
  });

  it("attaches running jobs to their owner and ignores finished ones", () => {
    const groups = ownerGroups(
      [run("web")],
      [job("j1"), job("j2", { status: "succeeded" }), job("j3", { owner: "agent:x" })],
    );
    expect(groups.find((g) => g.owner === "user")?.jobs.map((j) => j.id)).toEqual(["j1"]);
    // an owner with only a running job still shows up: stopping it cancels the job
    expect(groups.find((g) => g.owner === "agent:x")?.runs).toEqual([]);
  });

  it("is empty when nothing is active", () => {
    expect(ownerGroups([run("a", { state: "exited" })], [])).toEqual([]);
    expect(ownerCount([], [])).toBe(0);
  });

  it("counts owners the same way the sidebar badge does", () => {
    expect(ownerCount([run("a"), run("b", { owner: "agent:z" })], [job("j")])).toBe(2);
  });

  it("sorts runs by project then service", () => {
    const g = ownerGroups([run("z", { project: "a" }), run("a", { project: "b" }), run("a", { project: "a" })], []);
    expect(g[0]?.runs.map((r) => `${r.project}/${r.service}`)).toEqual(["a/a", "a/z", "b/a"]);
  });
});
