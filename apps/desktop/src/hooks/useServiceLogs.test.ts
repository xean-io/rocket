import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FollowMessage } from "@/lib/bindings";
import { useServiceLogs } from "./useServiceLogs";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage: (m: FollowMessage) => void = () => {};
  },
}));

interface FakeChannel {
  onmessage: (m: FollowMessage) => void;
}
let channels: FakeChannel[] = [];
let nextId = 0;

const logLine = (line: string): FollowMessage => ({
  kind: "event",
  event: { type: "log.line", time: "2026-01-01T00:00:00Z", line },
});

beforeEach(() => {
  channels = [];
  nextId = 0;
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string, args: { channel?: FakeChannel }) => {
    if (cmd === "follow_service_logs" && args.channel) {
      channels.push(args.channel);
      return `follow-${nextId++}`;
    }
    return undefined;
  });
});

const stops = () => invoke.mock.calls.filter(([cmd]) => cmd === "stop_follow").map(([, a]) => a.followId);

describe("useServiceLogs", () => {
  it("streams lines and caps the buffer", async () => {
    const { result } = renderHook(() => useServiceLogs("app", "web", true));
    await waitFor(() => expect(channels).toHaveLength(1));
    act(() => {
      for (let i = 0; i < 2100; i++) channels[0].onmessage(logLine(`l${i}`));
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(2000));
    expect(result.current.lines[0]).toBe("l100");
    expect(result.current.lines.at(-1)).toBe("l2099");
  });

  it("stops the follow when the selected service changes", async () => {
    const { rerender } = renderHook(({ s }) => useServiceLogs("app", s, true), {
      initialProps: { s: "web" },
    });
    await waitFor(() => expect(channels).toHaveLength(1));
    rerender({ s: "api" });
    await waitFor(() => expect(channels).toHaveLength(2));
    expect(stops()).toEqual(["follow-0"]);
  });

  it("stops the follow when the pane closes or unmounts", async () => {
    const { rerender, unmount } = renderHook(({ on }) => useServiceLogs("app", "web", on), {
      initialProps: { on: true },
    });
    await waitFor(() => expect(channels).toHaveLength(1));
    rerender({ on: false });
    await waitFor(() => expect(stops()).toEqual(["follow-0"]));
    rerender({ on: true });
    await waitFor(() => expect(channels).toHaveLength(2));
    unmount();
    await waitFor(() => expect(stops()).toEqual(["follow-0", "follow-1"]));
  });

  it("stops a follow that resolves after the pane already closed", async () => {
    let resolve!: (id: string) => void;
    invoke.mockImplementationOnce(() => new Promise<string>((r) => (resolve = r)));
    const { unmount } = renderHook(() => useServiceLogs("app", "web", true));
    unmount();
    resolve("late");
    await waitFor(() => expect(stops()).toEqual(["late"]));
  });

  it("reload clears the buffer and re-follows", async () => {
    const { result } = renderHook(() => useServiceLogs("app", "web", true));
    await waitFor(() => expect(channels).toHaveLength(1));
    act(() => channels[0].onmessage(logLine("old")));
    await waitFor(() => expect(result.current.lines).toEqual(["old"]));
    act(() => result.current.reload());
    expect(result.current.lines).toEqual([]);
    await waitFor(() => expect(channels).toHaveLength(2));
    expect(stops()).toEqual(["follow-0"]);
  });
});
