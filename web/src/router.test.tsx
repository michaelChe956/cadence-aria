import {
  RouterProvider,
  createMemoryHistory,
  type RouterHistory,
} from "@tanstack/react-router";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderHealthResponse } from "./api/types";
import { useProviderAvailabilityStore } from "./state/provider-availability-store";
import { createAppRouter, router } from "./router";

vi.mock("./app-shell", () => ({
  AppShell: () => <div data-testid="workbench-page">Workbench</div>,
}));

vi.mock("./pages/ChatWorkspacePage", () => ({
  ChatWorkspacePage: () => <div data-testid="chat-workspace-page">Chat Workspace</div>,
}));

vi.mock("./pages/CodingWorkspacePage", () => ({
  CodingWorkspacePage: () => (
    <div data-testid="coding-workspace-page">Coding Workspace</div>
  ),
}));

const originalActions = {
  load: useProviderAvailabilityStore.getState().load,
  recheck: useProviderAvailabilityStore.getState().recheck,
  reset: useProviderAvailabilityStore.getState().reset,
};

function blockedSnapshot(): ProviderHealthResponse {
  return {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: true,
    test_provider_enabled: false,
    providers: [
      {
        provider: "claude_code",
        display_name: "Claude Code",
        available: false,
        version: null,
        reason_code: "command_missing",
        reason: "Claude command missing",
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "Install Claude Code",
      },
      {
        provider: "codex",
        display_name: "Codex",
        available: false,
        version: null,
        reason_code: "command_missing",
        reason: "Codex command missing",
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "Install Codex",
      },
    ],
  };
}

function memoryRouter(path: string, history?: RouterHistory) {
  return createAppRouter(
    history ?? createMemoryHistory({ initialEntries: [path] }),
  );
}

describe("router", () => {
  beforeEach(() => {
    originalActions.reset();
    useProviderAvailabilityStore.setState(originalActions);
  });

  it("registers the coding workspace route", () => {
    expect(router.routesByPath["/workbench/coding/$attemptId"]).toBeDefined();
  });

  it("mounts one shared root guard and starts the initial load once", async () => {
    const load = vi.fn().mockImplementation(() => {
      useProviderAvailabilityStore.setState({ loadStatus: "loading" });
      return Promise.resolve();
    });
    useProviderAvailabilityStore.setState({ load });

    render(<RouterProvider router={memoryRouter("/workbench")} />);

    await waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("status")).toHaveTextContent(
      "正在检测 Claude Code 与 Codex",
    );
    expect(screen.queryByTestId("workbench-page")).not.toBeInTheDocument();
  });

  it.each([
    ["Workbench", "/workbench", "workbench-page"],
    ["Chat Workspace", "/workbench/workspace/session_0001", "chat-workspace-page"],
    ["Coding Workspace", "/workbench/coding/attempt_0001", "coding-workspace-page"],
  ])("does not let %s bypass the root guard", async (_name, path, pageTestId) => {
    const health = blockedSnapshot();
    useProviderAvailabilityStore.setState({
      snapshot: health,
      loadStatus: "loaded",
      generation: health.generation,
      stateStatus: health.state_status,
      stateError: health.state_error,
      realWorkflowBlocked: health.real_workflow_blocked,
      testProviderEnabled: health.test_provider_enabled,
    });

    render(<RouterProvider router={memoryRouter(path)} />);

    expect(
      await screen.findByRole("dialog", { name: "需要安装或修复 Provider" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId(pageTestId)).not.toBeInTheDocument();
  });
});
