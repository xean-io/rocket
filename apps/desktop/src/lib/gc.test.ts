import { describe, expect, it } from "vitest";
import { gcSummary } from "./gc";

describe("gcSummary", () => {
  it("says so when there is nothing to do", () => {
    expect(gcSummary({ actions: [] })).toBe("Nothing to clean up");
  });
  it("counts actions by kind", () => {
    const a = (action: string) => ({ action, project: "p", service: "s" });
    expect(gcSummary({ actions: [a("pruned"), a("pruned"), a("lost_job")] })).toBe(
      "Cleaned 3 items: 2 pruned, 1 lost job",
    );
    expect(gcSummary({ actions: [a("pruned")] })).toBe("Cleaned 1 item: 1 pruned");
  });
});
