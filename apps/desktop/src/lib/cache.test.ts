import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import type { Event, Job, JobsResult, Run, StatusResult } from "./bindings";
import { applyEvent, queryKeys, resync, upsertJob, upsertRun } from "./cache";

const run = (service: string, state: Run["state"] = "running"): Run => ({
  project: "app",
  service,
  env: "dev",
  kind: "run",
  state,
  health: "unknown",
});

const job = (id: string, status: Job["status"] = "running"): Job => ({
  id,
  project: "app",
  name: "build",
  kind: "pipeline",
  owner: "user",
  steps: [],
  status,
  started_at: "2026-01-01T00:00:00Z",
  duration_ms: 0,
});

const ev = (e: Partial<Event> & Pick<Event, "type">): Event => ({
  time: "2026-01-01T00:00:00Z",
  ...e,
});

function client() {
  const qc = new QueryClient();
  qc.setQueryData<StatusResult>(queryKeys.runs, { services: [run("api"), run("web")] });
  qc.setQueryData<JobsResult>(queryKeys.jobs, { jobs: [job("j1")] });
  return qc;
}

describe("upsert helpers", () => {
  it("replaces a run in place and appends unknown ones", () => {
    const runs = [run("api"), run("web")];
    expect(upsertRun(runs, run("api", "stopped")).map((r) => r.state)).toEqual([
      "stopped",
      "running",
    ]);
    expect(upsertRun(runs, run("db")).map((r) => r.service)).toEqual(["api", "web", "db"]);
    expect(runs[0]?.state).toBe("running"); // input untouched
  });

  it("puts new jobs first and replaces known ones", () => {
    const jobs = [job("j1")];
    expect(upsertJob(jobs, job("j2")).map((j) => j.id)).toEqual(["j2", "j1"]);
    expect(upsertJob(jobs, job("j1", "failed"))[0]?.status).toBe("failed");
  });
});

describe("applyEvent", () => {
  it("patches the cached runs on service.state", () => {
    const qc = client();
    applyEvent(qc, ev({ type: "service.state", run: run("api", "failed") }));
    const runs = qc.getQueryData<StatusResult>(queryKeys.runs)?.services;
    expect(runs?.map((r) => `${r.service}:${r.state}`)).toEqual(["api:failed", "web:running"]);
  });

  it("invalidates derived snapshots on service.state", () => {
    const qc = client();
    qc.setQueryData(queryKeys.summary, {});
    qc.setQueryData(queryKeys.ports, {});
    applyEvent(qc, ev({ type: "service.state", run: run("api") }));
    expect(qc.getQueryState(queryKeys.summary)?.isInvalidated).toBe(true);
    expect(qc.getQueryState(queryKeys.ports)?.isInvalidated).toBe(true);
    expect(qc.getQueryState(queryKeys.jobs)?.isInvalidated).toBe(false);
  });

  it("patches the cached jobs on job.state", () => {
    const qc = client();
    applyEvent(qc, ev({ type: "job.state", job: job("j1", "succeeded") }));
    applyEvent(qc, ev({ type: "job.state", job: job("j2") }));
    const jobs = qc.getQueryData<JobsResult>(queryKeys.jobs)?.jobs;
    expect(jobs?.map((j) => `${j.id}:${j.status}`)).toEqual(["j2:running", "j1:succeeded"]);
  });

  it("only invalidates ports on port events and ignores log events", () => {
    const qc = client();
    qc.setQueryData(queryKeys.ports, {});
    applyEvent(qc, ev({ type: "log.line", line: "hi" }));
    applyEvent(qc, ev({ type: "job.log", line: "hi" }));
    expect(qc.getQueryState(queryKeys.ports)?.isInvalidated).toBe(false);
    applyEvent(qc, ev({ type: "port.leased" }));
    expect(qc.getQueryState(queryKeys.ports)?.isInvalidated).toBe(true);
  });

  it("does not create a cache entry that was never loaded", () => {
    const qc = new QueryClient();
    applyEvent(qc, ev({ type: "service.state", run: run("api") }));
    expect(qc.getQueryData(queryKeys.runs)).toBeUndefined();
  });
});

describe("resync", () => {
  it("invalidates every cached query", () => {
    const qc = client();
    resync(qc);
    expect(qc.getQueryState(queryKeys.runs)?.isInvalidated).toBe(true);
    expect(qc.getQueryState(queryKeys.jobs)?.isInvalidated).toBe(true);
  });
});
