import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UpdateStatus } from "./bindings";
import {
  AUTO_CHECK_KEY,
  CHECK_INTERVAL_MS,
  FIRST_CHECK_DELAY_MS,
  createUpdateController,
  progressLabel,
  readAutoCheck,
  readLastChecked,
  staleDaemonMessage,
  startScheduler,
  writeAutoCheck,
  writeLastChecked,
} from "./updates";

const available = (version: string): UpdateStatus => ({
  available: true,
  version,
  current_version: "0.1.1",
  notes: "notes",
  date: null,
});
const upToDate: UpdateStatus = {
  available: false,
  version: null,
  current_version: "0.1.1",
  notes: null,
  date: null,
};

function harness(...answers: Array<UpdateStatus | Error>) {
  const queue = [...answers];
  const ui = { announce: vi.fn(), upToDate: vi.fn(), failed: vi.fn() };
  const check = vi.fn(async () => {
    const next = queue.length > 1 ? queue.shift()! : queue[0]!;
    if (next instanceof Error) throw next;
    return next;
  });
  const onChecked = vi.fn();
  const controller = createUpdateController({ check, ui, onChecked, now: () => 1234 });
  return { controller, ui, check, onChecked };
}

describe("update controller", () => {
  it("announces an available update and records when it checked", async () => {
    const h = harness(available("0.2.0"));
    await h.controller.check({ manual: false });
    expect(h.ui.announce).toHaveBeenCalledWith(available("0.2.0"));
    expect(h.onChecked).toHaveBeenCalledWith(1234);
  });

  it("Later suppresses that version for automatic checks until the next launch", async () => {
    const h = harness(available("0.2.0"));
    await h.controller.check({ manual: false });
    h.controller.later("0.2.0");
    await h.controller.check({ manual: false });
    await h.controller.check({ manual: false });
    expect(h.ui.announce).toHaveBeenCalledOnce();
  });

  it("a newer version is announced again after Later", async () => {
    const h = harness(available("0.2.0"), available("0.3.0"));
    await h.controller.check({ manual: false });
    h.controller.later("0.2.0");
    await h.controller.check({ manual: false });
    expect(h.ui.announce).toHaveBeenCalledTimes(2);
    expect(h.ui.announce).toHaveBeenLastCalledWith(available("0.3.0"));
  });

  it("a manual check announces even a dismissed version", async () => {
    const h = harness(available("0.2.0"));
    h.controller.later("0.2.0");
    await h.controller.check({ manual: true });
    expect(h.ui.announce).toHaveBeenCalledOnce();
  });

  it("says so when a manual check finds nothing, and stays quiet for automatic ones", async () => {
    const h = harness(upToDate);
    await h.controller.check({ manual: false });
    expect(h.ui.upToDate).not.toHaveBeenCalled();
    await h.controller.check({ manual: true });
    expect(h.ui.upToDate).toHaveBeenCalledWith(upToDate);
  });

  it("automatic checks swallow errors (offline is silent); manual ones report them", async () => {
    const h = harness(new Error("offline"));
    await h.controller.check({ manual: false });
    expect(h.ui.failed).not.toHaveBeenCalled();
    await h.controller.check({ manual: true });
    expect(h.ui.failed).toHaveBeenCalledOnce();
  });

  it("never runs two checks at once", async () => {
    let release!: (s: UpdateStatus) => void;
    const check = vi.fn(() => new Promise<UpdateStatus>((r) => (release = r)));
    const ui = { announce: vi.fn(), upToDate: vi.fn(), failed: vi.fn() };
    const controller = createUpdateController({ check, ui });
    const first = controller.check({ manual: false });
    await controller.check({ manual: true });
    expect(check).toHaveBeenCalledOnce();
    release(upToDate);
    await first;
  });
});

describe("scheduler", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("checks 10 s after launch, then every 6 h", async () => {
    const run = vi.fn();
    startScheduler({ run, enabled: () => true });
    expect(run).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS - 1);
    expect(run).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(run).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS);
    expect(run).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS);
    expect(run).toHaveBeenCalledTimes(3);
  });

  it("skips ticks while the setting is off and resumes when it is turned back on", async () => {
    let on = false;
    const run = vi.fn();
    startScheduler({ run, enabled: () => on });
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS);
    expect(run).not.toHaveBeenCalled();
    on = true;
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("stop cancels pending and future checks", async () => {
    const run = vi.fn();
    const stop = startScheduler({ run, enabled: () => true });
    stop();
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS * 2);
    expect(run).not.toHaveBeenCalled();
    const run2 = vi.fn();
    const stop2 = startScheduler({ run: run2, enabled: () => true });
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS);
    stop2();
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS * 2);
    expect(run2).toHaveBeenCalledTimes(1);
  });
});

describe("persisted settings", () => {
  const memory = () => {
    const m = new Map<string, string>();
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
  };
  const broken = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };

  it("auto-check defaults to on and round-trips", () => {
    const s = memory();
    expect(readAutoCheck(s)).toBe(true);
    writeAutoCheck(false, s);
    expect(s.getItem(AUTO_CHECK_KEY)).toBe("false");
    expect(readAutoCheck(s)).toBe(false);
    writeAutoCheck(true, s);
    expect(readAutoCheck(s)).toBe(true);
  });

  it("last-checked round-trips and ignores garbage", () => {
    const s = memory();
    expect(readLastChecked(s)).toBeNull();
    writeLastChecked(1700000000000, s);
    expect(readLastChecked(s)).toBe(1700000000000);
    s.setItem("rocket.updates.lastChecked", "nope");
    expect(readLastChecked(s)).toBeNull();
  });

  it("survives unavailable storage", () => {
    expect(readAutoCheck(broken)).toBe(true);
    expect(readLastChecked(broken)).toBeNull();
    expect(() => writeAutoCheck(false, broken)).not.toThrow();
    expect(() => writeLastChecked(1, broken)).not.toThrow();
    expect(readAutoCheck(null)).toBe(true);
  });
});

describe("progressLabel", () => {
  it("shows a percentage when the size is known, bytes otherwise", () => {
    expect(progressLabel(512, 2048)).toBe("25%");
    expect(progressLabel(4096, 2048)).toBe("100%");
    expect(progressLabel(1536 * 1024, null)).toBe("1.5 MB");
    expect(progressLabel(0, null)).toBe("Starting…");
  });
});

describe("staleDaemonMessage", () => {
  it("names both versions", () => {
    expect(staleDaemonMessage({ running: "0.1.1", bundled: "0.2.0" })).toBe(
      "Daemon is running 0.1.1; restart it to use 0.2.0.",
    );
  });
});
