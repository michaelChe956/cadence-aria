import {
  RouterProvider,
  createMemoryHistory,
  type RouterHistory,
} from "@tanstack/react-router";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CodingAttemptAddress, ProviderHealthResponse } from "./api/types";
import { useProviderAvailabilityStore } from "./state/provider-availability-store";
import { createAppRouter, router } from "./router";

vi.mock("./app-shell", () => ({
  AppShell: () => <div data-testid="workbench-page">Workbench</div>,
}));

vi.mock("./pages/ChatWorkspacePage", () => ({
  ChatWorkspacePage: () => <div data-testid="chat-workspace-page">Chat Workspace</div>,
}));

vi.mock("./pages/CodingWorkspacePage", () => ({
  CodingWorkspacePage: ({ address }: { address: CodingAttemptAddress }) => (
    <div
      data-testid="coding-workspace-page"
      data-project-id={address.projectId}
      data-issue-id={address.issueId}
      data-attempt-id={address.attemptId}
    >
      Coding Workspace
    </div>
  ),
}));

vi.mock("./pages/LegacyCodingWorkspaceRedirect", () => ({
  LegacyCodingWorkspaceRedirect: ({
    attemptId,
    onResolved,
    onBack,
  }: {
    attemptId: string;
    onResolved: (address: CodingAttemptAddress) => void;
    onBack: () => void;
  }) => (
    <div data-testid="legacy-coding-workspace-page" data-attempt-id={attemptId}>
      Legacy Coding Workspace
      <button
        type="button"
        onClick={() =>
          onResolved({
            projectId: "project/with space",
            issueId: "issue?#with space",
            attemptId,
          })
        }
      >
        解析旧地址
      </button>
      <button type="button" onClick={onBack}>
        返回 Workbench
      </button>
    </div>
  ),
}));

vi.mock("./pages/ImageCreatePage", () => ({
  ImageCreatePage: ({ sessionId }: { sessionId?: string }) => (
    <div data-testid="image-create-page" data-session-id={sessionId}>
      Image Create
    </div>
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
    expect(
      router.routesByPath[
        "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId"
      ],
    ).toBeDefined();
    expect(
      (router.routesByPath as Record<string, unknown>)[
        "/workbench/coding/$attemptId"
      ],
    ).toBeDefined();
    expect(router.routesByPath["/workbench/workspace/$sessionId"]).toBeDefined();
  });

  it("replaces a legacy coding workspace address with the scoped address", async () => {
    const attemptId = "coding attempt/%1";
    const health = {
      ...blockedSnapshot(),
      real_workflow_blocked: false,
    };
    useProviderAvailabilityStore.setState({
      snapshot: health,
      loadStatus: "loaded",
      generation: health.generation,
      stateStatus: health.state_status,
      stateError: health.state_error,
      realWorkflowBlocked: health.real_workflow_blocked,
      testProviderEnabled: health.test_provider_enabled,
    });
    const history = createMemoryHistory({
      initialEntries: [`/workbench/coding/${encodeURIComponent(attemptId)}`],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    const legacyPage = await screen.findByTestId("legacy-coding-workspace-page");
    expect(legacyPage).toHaveAttribute("data-attempt-id", attemptId);
    await userEvent.click(
      screen.getByRole("button", { name: "解析旧地址" }),
    );

    const page = await screen.findByTestId("coding-workspace-page");
    expect(page).toHaveAttribute("data-project-id", "project/with space");
    expect(page).toHaveAttribute("data-issue-id", "issue?#with space");
    expect(page).toHaveAttribute("data-attempt-id", attemptId);
    expect(history.location.pathname).toBe(
      `/workbench/projects/${encodeURIComponent("project/with space")}/issues/${encodeURIComponent("issue?#with space")}/coding/${encodeURIComponent(attemptId)}`,
    );
    expect(history.length).toBe(1);
  });

  it("returns from the legacy coding workspace address to Workbench", async () => {
    const health = {
      ...blockedSnapshot(),
      real_workflow_blocked: false,
    };
    useProviderAvailabilityStore.setState({
      snapshot: health,
      loadStatus: "loaded",
      generation: health.generation,
      stateStatus: health.state_status,
      stateError: health.state_error,
      realWorkflowBlocked: health.real_workflow_blocked,
      testProviderEnabled: health.test_provider_enabled,
    });

    render(
      <RouterProvider
        router={memoryRouter("/workbench/coding/coding_attempt_0001")}
      />,
    );

    await userEvent.click(
      await screen.findByRole("button", { name: "返回 Workbench" }),
    );
    expect(await screen.findByTestId("workbench-page")).toBeInTheDocument();
  });

  it("passes the complete coding attempt address from route params to the page", async () => {
    const address = {
      projectId: "project/with space",
      issueId: "issue?#with space",
      attemptId: "coding attempt/%1",
    };
    const health = {
      ...blockedSnapshot(),
      real_workflow_blocked: false,
    };
    useProviderAvailabilityStore.setState({
      snapshot: health,
      loadStatus: "loaded",
      generation: health.generation,
      stateStatus: health.state_status,
      stateError: health.state_error,
      realWorkflowBlocked: health.real_workflow_blocked,
      testProviderEnabled: health.test_provider_enabled,
    });

    render(
      <RouterProvider
        router={memoryRouter(
          `/workbench/projects/${encodeURIComponent(address.projectId)}/issues/${encodeURIComponent(address.issueId)}/coding/${encodeURIComponent(address.attemptId)}`,
        )}
      />,
    );

    const page = await screen.findByTestId("coding-workspace-page");
    expect(page).toHaveAttribute("data-project-id", address.projectId);
    expect(page).toHaveAttribute("data-issue-id", address.issueId);
    expect(page).toHaveAttribute("data-attempt-id", address.attemptId);
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

  it("renders image-create routes outside the provider availability guard", async () => {
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

    render(<RouterProvider router={memoryRouter("/image-create")} />);

    expect(await screen.findByTestId("image-create-page")).toBeInTheDocument();
    expect(
      screen.queryByRole("dialog", { name: "需要安装或修复 Provider" }),
    ).not.toBeInTheDocument();
  });

  it("passes the image-create session id from the independent route", async () => {
    render(
      <RouterProvider
        router={memoryRouter("/image-create/session%20with%20spaces")}
      />,
    );

    expect(await screen.findByTestId("image-create-page")).toHaveAttribute(
      "data-session-id",
      "session with spaces",
    );
  });

  it.each([
    ["Workbench", "/workbench", "workbench-page"],
    ["Chat Workspace", "/workbench/workspace/session_0001", "chat-workspace-page"],
    [
      "Coding Workspace",
      "/workbench/projects/project_0001/issues/issue_0001/coding/attempt_0001",
      "coding-workspace-page",
    ],
    [
      "Legacy Coding Workspace",
      "/workbench/coding/attempt_0001",
      "legacy-coding-workspace-page",
    ],
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
