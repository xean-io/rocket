import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import type { Job, JobsResult } from "./bindings";
import { applyEvent, queryKeys, upsertJob } from "./cache";
import { fetchJobsGuarded, mergeJobs, rerunRequest, sortJobs, stepState } from "./jobs";

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

describe("upsertJob terminal-wins rule", () => {
  it("never overwrites a terminal job with a late running snapshot", () => {
    const jobs = [job("j1", { status: "succeeded", exit_code: 0, duration_ms: 900 })];
    const next = upsertJob(jobs, job("j1", { status: "running" }));
    expect(next[0]?.status).toBe("succeeded");
    expect(next[0]?.duration_ms).toBe(900);
  });

  it("still lets a terminal job replace a running one, and one terminal replace another", () => {
    expect(upsertJob([job("j1")], job("j1", { status: "failed" }))[0]?.status).toBe("failed");
    expect(
      upsertJob([job("j1", { status: "failed" })], job("j1", { status: "canceled" }))[0]?.status,
    ).toBe("canceled");
  });

  it("keeps the list newest first", () => {
    const jobs = [job("a", { started_at: "2026-01-01T00:00:00Z" })];
    const next = upsertJob(jobs, job("b", { started_at: "2026-01-02T00:00:00Z" }));
    expect(next.map((j) => j.id)).toEqual(["b", "a"]);
  });
});

describe("job.state events", () => {
  const seeded = (jobs: Job[]) => {
    const qc = new QueryClient();
    qc.setQueryData<JobsResult>(queryKeys.jobs, { jobs });
    return qc;
  };
  const stateEvent = (j: Job) => ({ type: "job.state", time: "2026-01-01T00:00:00Z", job: j });

  it("a late running event does not resurrect a finished job", () => {
    const qc = seeded([job("j1", { status: "failed" })]);
    applyEvent(qc, stateEvent(job("j1", { status: "running" })));
    expect(qc.getQueryData<JobsResult>(queryKeys.jobs)?.jobs[0]?.status).toBe("failed");
  });

  it("patches only the status when the event carries no job", () => {
    const qc = seeded([job("j1")]);
    applyEvent(qc, { type: "job.state", time: "x", job_id: "j1", status: "succeeded" });
    expect(qc.getQueryData<JobsResult>(queryKeys.jobs)?.jobs[0]?.status).toBe("succeeded");
    applyEvent(qc, { type: "job.state", time: "x", job_id: "j1", status: "running" });
    expect(qc.getQueryData<JobsResult>(queryKeys.jobs)?.jobs[0]?.status).toBe("succeeded");
  });
});

describe("jobs list refresh (stale-snapshot guard)", () => {
  it("keeps the event-applied state when an event lands while the request is in flight", async () => {
    const qc = new QueryClient();
    qc.setQueryData<JobsResult>(queryKeys.jobs, { jobs: [job("j1")] });
    let release!: (r: JobsResult) => void;
    const pending = fetchJobsGuarded(qc, () => new Promise<JobsResult>((r) => (release = r)));
    applyEvent(qc, { type: "job.state", time: "x", job: job("j1", { status: "succeeded" }) });
    release({ jobs: [job("j1", { status: "running" })] }); // older than the event
    const result = await pending;
    expect(result.jobs[0]?.status).toBe("succeeded");
  });

  it("returns the response untouched when nothing happened meanwhile", async () => {
    const qc = new QueryClient();
    const result = await fetchJobsGuarded(qc, async () => ({ jobs: [job("j2"), job("j1")] }));
    expect(result.jobs.map((j) => j.id)).toEqual(["j2", "j1"]);
  });

  it("never lets a response turn a terminal cached job back into running", async () => {
    const qc = new QueryClient();
    qc.setQueryData<JobsResult>(queryKeys.jobs, { jobs: [job("j1", { status: "canceled" })] });
    const result = await fetchJobsGuarded(qc, async () => ({ jobs: [job("j1")] }));
    expect(result.jobs[0]?.status).toBe("canceled");
  });
});

describe("mergeJobs / sortJobs", () => {
  it("sorts by start time then id, newest first", () => {
    const sorted = sortJobs([
      job("a", { started_at: "2026-01-01T00:00:00Z" }),
      job("c", { started_at: "2026-01-02T00:00:00Z" }),
      job("b", { started_at: "2026-01-02T00:00:00Z" }),
    ]);
    expect(sorted.map((j) => j.id)).toEqual(["c", "b", "a"]);
  });

  it("takes the incoming list but retains terminal states", () => {
    const merged = mergeJobs(
      [job("j1", { status: "succeeded" })],
      [job("j1"), job("j2")],
    );
    expect(merged.map((j) => `${j.id}:${j.status}`).sort()).toEqual(["j1:succeeded", "j2:running"]);
  });
});

describe("rerunRequest", () => {
  it("preserves env, profiles and args of a pipeline", () => {
    const req = rerunRequest(
      job("j1", { name: "test", env: "staging", profiles: ["db", "cache"], args: ["--fast"] }),
    );
    expect(req).toEqual({
      project: "app",
      kind: "pipeline",
      name: "test",
      env: "staging",
      profiles: ["db", "cache"],
      args: ["--fast"],
    });
  });

  it("omits empty lists and an unset env", () => {
    const req = rerunRequest(job("j1", { profiles: [], args: [] }));
    expect(req).toEqual({ project: "app", kind: "pipeline", name: "build" });
  });

  it("targets the deploy env, drops profiles and keeps args", () => {
    const req = rerunRequest(
      job("j2", { kind: "deploy", name: "prod", env: "prod", profiles: ["x"], args: ["--dry"] }),
    );
    expect(req).toEqual({ project: "app", kind: "deploy", name: "prod", env: "prod", args: ["--dry"] });
  });

  it("falls back to the job name when an old deploy has no env", () => {
    const req = rerunRequest(job("j3", { kind: "deploy", name: "prod" }));
    expect(req.env).toBe("prod");
    expect(req.name).toBe("prod");
  });
});

describe("stepState", () => {
  const steps = [{ task: "a" }, { task: "b" }, { run: "c" }];
  it("marks earlier steps done and the current one active while running", () => {
    const j = job("j", { steps, step: 2 });
    expect([0, 1, 2].map((i) => stepState(j, i))).toEqual(["done", "current", "pending"]);
  });
  it("marks everything done when the job succeeded", () => {
    const j = job("j", { steps, step: 3, status: "succeeded" });
    expect([0, 1, 2].map((i) => stepState(j, i))).toEqual(["done", "done", "done"]);
  });
  it("flags the step a failed job stopped at", () => {
    const j = job("j", { steps, step: 2, status: "failed" });
    expect([0, 1, 2].map((i) => stepState(j, i))).toEqual(["done", "failed", "pending"]);
  });
});

describe("jobDuration", () => {
  const started = Date.parse("2026-01-01T00:00:00Z");
  it("ticks for a running job", async () => {
    const { jobDuration } = await import("./jobs");
    expect(jobDuration(job("j"), started + 5000)).toBe(5000);
  });
  it("uses the recorded duration once finished", async () => {
    const { jobDuration } = await import("./jobs");
    expect(jobDuration(job("j", { status: "failed", duration_ms: 1200 }), started + 99999)).toBe(1200);
  });
});
