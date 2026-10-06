import { RocketError } from "./rocket";
import type { UpResult } from "./bindings";

/** One readable line for a failed command (used by toasts). */
export function describeError(e: unknown): { title: string; detail?: string } {
  if (e instanceof RocketError) {
    switch (e.code) {
      case "unreachable":
      case "daemon_info":
      case "daemon_start_timeout":
      case "launch":
        return { title: "rocketd is not reachable", detail: e.message };
      case "unauthorized":
        return { title: "Daemon token rejected", detail: "Reconnecting with the current token." };
      case "not_found":
        return { title: "Not found", detail: e.message };
      case "conflict":
        return { title: "Conflict", detail: e.message };
      case "invalid":
        return { title: "Invalid request", detail: e.message };
      case "confirmation_required":
        return { title: "Confirmation required", detail: e.message };
      case "endpoint":
        return { title: "Daemon too old", detail: "Update Rocket to use this feature." };
      default:
        return { title: "Request failed", detail: e.message };
    }
  }
  return { title: "Something went wrong", detail: e instanceof Error ? e.message : String(e) };
}

/** Failed services of an `up`/`restart`, one per line; `null` when all went well. */
export function failureSummary(result: UpResult): string | null {
  const failed = result.services.filter((s) => s.action === "failed" || s.state === "failed");
  if (failed.length === 0) return null;
  return failed.map((s) => `${s.service}: ${s.error || s.state}`).join("\n");
}
