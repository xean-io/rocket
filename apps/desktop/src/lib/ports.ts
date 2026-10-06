// Port map helpers (port of PortsView.swift).
import type { Conflict, PortInfo } from "./bindings";

export const isRemap = (c: Conflict) => c.kind === "port_remapped";

/** The remap record of a lease: same project, service and port name. */
export function remapFor(port: PortInfo, conflicts: readonly Conflict[]): Conflict | undefined {
  return conflicts.find(
    (c) => isRemap(c) && c.project === port.project && c.service === port.service && c.port_name === port.port_name,
  );
}

/** Ports a service wants but cannot get. */
export const blockedConflicts = (conflicts: readonly Conflict[]) => conflicts.filter((c) => !isRemap(c));

export function portsSummary(ports: readonly PortInfo[], conflicts: readonly Conflict[]): string {
  const remapped = conflicts.filter(isRemap).length;
  return `${ports.length} leased · ${remapped} remapped · ${blockedConflicts(conflicts).length} blocked`;
}

export const portUrl = (port: number) => `http://127.0.0.1:${port}`;

/** The amber "from N" tag shown next to a remapped port. */
export const remapLabel = (remap: Conflict) => `from ${remap.default}`;
