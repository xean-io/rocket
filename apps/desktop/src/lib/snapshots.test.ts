import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import type { Event, Job, JobsResult, Run, StatusResult } from "./bindings";
import { applyEvent, queryKeys } from "./cache";
import { currentRevision, guardedFetch } from "./snapshots";

const run = (state: Run["state"]): Run => ({
  project: "app",
  service: "web",
  env: "dev",
  kind: "run",
  state,
  health: "unknown",
});

const job = (status: Job["status"]): Job => ({
  id: "j1",
  project: "app",
  name: "build",
  kind: "pipeline",
  owner: "user",
  steps: [],
  status,
  started_at: "2026-01-01T00:00:00Z",
  duration_ms: 0,
});

const ev = (e: Partial<Event> & Pick<Event, "type">): Event => ({ time: "2026-01-01T00:00:00Z", ...e });

describe("guardedFetch", () => {
  it("returns the snapshot when nothing changed meanwhile", async () => {
    const qc = new QueryClient();
    const data = await guardedFetch(qc, queryKeys.runs, async () => ({ services: [run("stopped")] }));
    expect(data.services[0].state).toBe("stopped");
  });

  it("keeps newer event-applied state over an older in-flight snapshot", async () => {
    const qc = new QueryClient();
    qc.setQueryData<StatusResult>(queryKeys.runs, { services: [run("stopped")] });
    let release!: (v: StatusResult) => void;
    const inflight = guardedFetch(
      qc,
      queryKeys.runs,
      () => new Promise<StatusResult>((r) => (release = r)),
    );
    // An event lands while the snapshot request is on the wire.
    applyEvent(qc, ev({ type: "service.state", run: run("running") }));
    release({ services: [run("stopped")] });
    const result = await inflight;
    expect(result.services[0].state).toBe("running");
    expect(qc.getQueryData<StatusResult>(queryKeys.runs)?.services[0].state).toBe("running");
  });

  it("guards the jobs snapshot the same way", async () => {
    const qc = new QueryClient();
    qc.setQueryData<JobsResult>(queryKeys.jobs, { jobs: [job("running")] });
    let release!: (v: JobsResult) => void;
    const inflight = guardedFetch(qc, queryKeys.jobs, () => new Promise<JobsResult>((r) => (release = r)));
    applyEvent(qc, ev({ type: "job.state", job: job("succeeded") }));
    release({ jobs: [job("running")] });
    expect((await inflight).jobs[0].status).toBe("succeeded");
  });

  it("accepts a snapshot started after the last event", async () => {
    const qc = new QueryClient();
    qc.setQueryData<StatusResult>(queryKeys.runs, { services: [run("stopped")] });
    applyEvent(qc, ev({ type: "service.state", run: run("running") }));
    const before = currentRevision(qc, queryKeys.runs);
    const data = await guardedFetch(qc, queryKeys.runs, async () => ({ services: [run("stopped")] }));
    expect(data.services[0].state).toBe("stopped");
    expect(currentRevision(qc, queryKeys.runs)).toBe(before);
  });

  it("patches per-project summaries and bumps their revision", () => {
    const qc = new QueryClient();
    const key = queryKeys.projectSummary("app");
    qc.setQueryData(key, { services: [run("stopped")], jobs: [], conflicts: [] });
    const before = currentRevision(qc, key);
    applyEvent(qc, ev({ type: "service.state", run: run("running") }));
    expect(currentRevision(qc, key)).toBeGreaterThan(before);
    expect(qc.getQueryData<{ services: Run[] }>(key)?.services[0].state).toBe("running");
  });
});
