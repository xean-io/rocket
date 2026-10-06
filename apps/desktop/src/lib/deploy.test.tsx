import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DeployDialog } from "@/components/DeployDialog";
import type { Job } from "./bindings";
import { startJobOrConfirm } from "./deploy";
import { useNav } from "./nav";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke, Channel: class {} }));
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn() }) }));

const job: Job = {
  id: "j1",
  project: "shop",
  name: "prod",
  kind: "deploy",
  owner: "user",
  steps: [],
  status: "running",
  started_at: "2026-01-01T00:00:00Z",
  duration_ms: 0,
};

function renderDialog() {
  const qc = new QueryClient();
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <DeployDialog />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  useNav.setState({ pendingDeploy: null });
});

describe("deploy confirmation", () => {
  it("renders nothing without a pending deploy", () => {
    renderDialog();
    expect(screen.queryByText(/Deploy to/)).toBeNull();
  });

  it("Cancel sends nothing and closes the dialog", async () => {
    useNav.getState().requestDeploy({ project: "shop", env: "prod" });
    renderDialog();
    expect(await screen.findByText("Deploy to prod?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(invoke).not.toHaveBeenCalled();
    await waitFor(() => expect(useNav.getState().pendingDeploy).toBeNull());
  });

  it("Deploy calls start_job with yes: true", async () => {
    invoke.mockResolvedValue(job);
    useNav.getState().requestDeploy({ project: "shop", env: "prod" });
    renderDialog();
    await userEvent.click(await screen.findByRole("button", { name: "Deploy" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    expect(invoke).toHaveBeenCalledWith("start_job", {
      request: { project: "shop", kind: "deploy", name: "prod", env: "prod", yes: true },
    });
    await waitFor(() => expect(useNav.getState().pendingDeploy).toBeNull());
  });

  it("a 428 from the daemon opens the same dialog and keeps the request", async () => {
    invoke.mockRejectedValueOnce({
      code: "confirmation_required",
      message: "deploy needs confirmation",
      status: 428,
    });
    const request = { project: "shop", kind: "deploy" as const, name: "prod", env: "prod" };
    expect(await startJobOrConfirm(request)).toBeNull();
    expect(useNav.getState().pendingDeploy).toEqual({ project: "shop", env: "prod", request });

    invoke.mockResolvedValue(job);
    renderDialog();
    expect(await screen.findByText("Deploy to prod?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Deploy" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenLastCalledWith("start_job", { request: { ...request, yes: true } }),
    );
  });

  it("other errors are rethrown, not turned into a dialog", async () => {
    invoke.mockRejectedValueOnce({ code: "invalid", message: "bad env", status: 400 });
    await expect(
      startJobOrConfirm({ project: "shop", kind: "pipeline", name: "test" }),
    ).rejects.toMatchObject({ code: "invalid" });
    expect(useNav.getState().pendingDeploy).toBeNull();
  });
});
