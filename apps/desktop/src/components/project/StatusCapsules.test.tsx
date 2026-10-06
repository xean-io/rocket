import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Run } from "@/lib/bindings";
import { StatusCapsules } from "./StatusCapsules";

const run = (service: string, state: Run["state"]): Run => ({
  project: "app",
  service,
  env: "dev",
  kind: "run",
  state,
  health: "unknown",
});

describe("StatusCapsules", () => {
  it("shows running count and only the non-zero other capsules", () => {
    render(<StatusCapsules runs={[run("a", "running"), run("b", "running"), run("c", "stopped")]} />);
    expect(screen.getByTitle("2 running")).toBeInTheDocument();
    expect(screen.queryByTitle(/changing/)).toBeNull();
    expect(screen.queryByTitle(/down/)).toBeNull();
  });

  it("counts changing and down services", () => {
    render(
      <StatusCapsules
        runs={[run("a", "starting"), run("b", "failed"), run("c", "dead"), run("d", "exited")]}
        busy
      />,
    );
    expect(screen.getByTitle("0 running")).toBeInTheDocument();
    expect(screen.getByTitle("1 changing")).toBeInTheDocument();
    expect(screen.getByTitle("3 down")).toBeInTheDocument();
    expect(screen.getByText("Working")).toBeInTheDocument();
  });
});
