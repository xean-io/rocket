import { describe, expect, it } from "vitest";
import type { Conflict, PortInfo } from "./bindings";
import { blockedConflicts, portsSummary, remapFor } from "./ports";

const conflict = (over: Partial<Conflict> = {}): Conflict => ({
  kind: "port_remapped",
  project: "app",
  service: "web",
  port_name: "http",
  port: 3001,
  default: 3000,
  remappable: true,
  detail: "3000 busy (node, pid 42)",
  ...over,
});
const port = (over: Partial<PortInfo> = {}): PortInfo => ({
  port: 3001,
  project: "app",
  service: "web",
  port_name: "http",
  created_at: "2026-01-01T00:00:00Z",
  ...over,
});

describe("remapFor", () => {
  it("matches a remapped lease and exposes the original port", () => {
    const r = remapFor(port(), [conflict()]);
    expect(r?.default).toBe(3000);
  });
  it("ignores blocked conflicts and other services", () => {
    expect(remapFor(port(), [conflict({ kind: "port_busy" })])).toBeUndefined();
    expect(remapFor(port(), [conflict({ service: "api" })])).toBeUndefined();
    expect(remapFor(port(), [conflict({ port_name: "admin" })])).toBeUndefined();
    expect(remapFor(port(), [conflict({ project: "other" })])).toBeUndefined();
  });
});

describe("blockedConflicts / portsSummary", () => {
  it("separates blocked from remapped", () => {
    const cs = [conflict(), conflict({ kind: "port_busy", service: "db" })];
    expect(blockedConflicts(cs).map((c) => c.service)).toEqual(["db"]);
    expect(portsSummary([port()], cs)).toBe("1 leased · 1 remapped · 1 blocked");
  });
});

describe("remapLabel", () => {
  it("names the port the service originally asked for", async () => {
    const { remapLabel } = await import("./ports");
    expect(remapLabel(conflict())).toBe("from 3000");
  });
});
