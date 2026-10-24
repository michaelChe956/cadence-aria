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
import { useWorkspaceSessionObservers } from "./hooks/useWorkspaceSessionObservers";
import type { CockpitInboxItem } from "./state/workspace-cockpit-projection";
import { useWorkbenchOverlayDirectory } from "./state/workbench-overlay-store";
import {
  closeWorkbenchOverlayPatch,
  mergeWorkbenchSearch,
} from "./workbench-search";

vi.mock("./app-shell", () => ({
  AppShell: ({
    onDrawerFocusChange,
  }: {
    onDrawerFocusChange?: (entityKey: string | null) => void;
  }) => (
    <div data-testid="workbench-page">
      Workbench
      {onDrawerFocusChange ? (
        <button
          type="button"
          onClick={() => onDrawerFocusChange("issue:issue_0001:issue_0001")}
        >
          聚焦实体
        </button>
      ) : null}
    </div>
  ),
}));

vi.mock("./hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: vi.fn(),
}));

vi.mock("./pages/ChatWorkspacePage", () => ({
  ChatWorkspacePage: ({
    onOpenSession,
    onOpenInfoCoding,
  }: {
    onOpenSession: (sessionId: string) => void;
    onOpenInfoCoding?: (address: {
      projectId: string;
      issueId: string;
      attemptId: string;
    }) => void;
  }) => (
    <div data-testid="chat-workspace-page">
      Chat Workspace
      <button type="button" onClick={() => onOpenSession("session_parent")}>
        返回父会话
      </button>
      {onOpenInfoCoding ? (
        <button
          type="button"
          onClick={() =>
            onOpenInfoCoding({
              projectId: "project_1",
              issueId: "issue_1",
              attemptId: "coding_attempt_001",
            })
          }
        >
          查看 Coding Workspace
        </button>
      ) : null}
    </div>
  ),
}));

vi.mock("./pages/CodingWorkspacePage", () => ({
  CodingWorkspacePage: ({
    address,
    onBack,
    onOpenPlanOverview,
  }: {
    address: CodingAttemptAddress;
    onBack: () => void;
    onOpenPlanOverview?: (planId: string) => void;
  }) => (
    <div
      data-testid="coding-workspace-page"
      data-project-id={address.projectId}
      data-issue-id={address.issueId}
      data-attempt-id={address.attemptId}
    >
      Coding Workspace
      <button type="button" onClick={onBack}>
        返回
      </button>
      {onOpenPlanOverview ? (
        <button type="button" onClick={() => onOpenPlanOverview("issue_plan_0001")}>
          查看全部 WI
        </button>
      ) : null}
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

const mockedUseWorkspaceSessionObservers = vi.mocked(useWorkspaceSessionObservers);

function gateItem(sessionId: string): CockpitInboxItem {
  return {
    id: `${sessionId}:gate`,
    kind: "gate",
    severity: 3,
    title: "需要人工确认",
    summary: "等待处理",
    triage: false,
    source: "gate",
    createdAt: null,
    gate: null,
    inlineError: null,
      choice: null,
  };
}

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
    useWorkbenchOverlayDirectory
      .getState()
      .resetWorkbenchOverlayDirectory();
    mockedUseWorkspaceSessionObservers.mockReturnValue({
      records: [],
      inbox: [],
      countedInbox: [],
      watchedSessionIds: [],
      watchSession: vi.fn(),
        codingAttemptForSession: () => null,
      displayItems: [],
      actionableCount: 0,
      notificationCandidates: [],
    });
  });

  it("navigates an inbox item from an unguarded root page to its workspace", async () => {
    const sessionId = "session_inbox_001";
    const history = createMemoryHistory({ initialEntries: ["/image-create"] });
    mockedUseWorkspaceSessionObservers.mockReturnValue({
      records: [],
      inbox: [gateItem(sessionId)],
      countedInbox: [gateItem(sessionId)],
      watchedSessionIds: [sessionId],
      watchSession: vi.fn(),
        codingAttemptForSession: () => null,
      displayItems: [gateItem(sessionId)],
      actionableCount: 1,
      notificationCandidates: [],
    });

    render(<RouterProvider router={createAppRouter(history)} />);
    await userEvent.click(await screen.findByRole("button", { name: "去处理" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe(`/workbench/workspace/${sessionId}`);
    });
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

  it("navigates a parent session through the existing workspace route", async () => {
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
      initialEntries: ["/workbench/workspace/child_001"],
    });

    render(<RouterProvider router={createAppRouter(history)} />);
    await userEvent.click(await screen.findByRole("button", { name: "返回父会话" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe("/workbench/workspace/session_parent");
    });
  });

  it("navigates coding final-confirm info to the scoped coding workspace route", async () => {
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
      initialEntries: ["/workbench/workspace/session_001"],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    await userEvent.click(
      await screen.findByRole("button", { name: "查看 Coding Workspace" }),
    );

    await waitFor(() => {
      expect(history.location.pathname).toBe(
        "/workbench/projects/project_1/issues/issue_1/coding/coding_attempt_001",
      );
    });
    const page = await screen.findByTestId("coding-workspace-page");
    expect(page).toHaveAttribute("data-project-id", "project_1");
    expect(page).toHaveAttribute("data-issue-id", "issue_1");
    expect(page).toHaveAttribute("data-attempt-id", "coding_attempt_001");
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

  it("wraps image-create routes in the global cockpit shell", async () => {
    const health = {
      ...blockedSnapshot(),
      test_provider_enabled: true,
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

    render(<RouterProvider router={memoryRouter("/image-create")} />);

    expect(await screen.findByTestId("image-create-page")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-shell")).toBeInTheDocument();
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


  // ── UI-1：/workbench 抽屉定位契约（overlay search）──

  function loadAvailableProviders() {
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
  }

  function seedPlanAddress() {
    useWorkbenchOverlayDirectory
      .getState()
      .seedWorkbenchOverlayAddress("issue_plan_0001", {
        projectId: "project_0001",
        issueId: "issue_0001",
      });
  }

  it("merges workbench search patches without dropping unrelated fields", () => {
    // 硬规则：任何写入必须合并现有 search（含关闭 patch 只清 overlay 字段）。
    const base = {
      focus: "work_item_group:issue_0001:issue_plan_0001",
      overlay: "plan-overview" as const,
      planId: "issue_plan_0001",
      attemptId: "coding_attempt_web_0001",
      gateId: "gate_1",
    };
    expect(mergeWorkbenchSearch(base, { focus: undefined })).toEqual({
      overlay: "plan-overview",
      planId: "issue_plan_0001",
      attemptId: "coding_attempt_web_0001",
      gateId: "gate_1",
    });
    expect(
      mergeWorkbenchSearch(base, {
        attemptId: "coding_attempt_busi_0001",
        gateId: undefined,
      }),
    ).toEqual({
      focus: "work_item_group:issue_0001:issue_plan_0001",
      overlay: "plan-overview",
      planId: "issue_plan_0001",
      attemptId: "coding_attempt_busi_0001",
    });
    // 契约外字段不透传（防跨路由串味）。
    expect(
      mergeWorkbenchSearch(
        { focus: "f", planId: "p" },
        closeWorkbenchOverlayPatch(),
      ),
    ).toEqual({ focus: "f" });
  });

  it("replaces a legacy plan overview link with the normalized workbench overlay address", async () => {
    loadAvailableProviders();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("plan aggregate not configured for this test");
      }),
    );
    const history = createMemoryHistory({
      initialEntries: [
        "/workbench/projects/project_0001/issues/issue_0001/coding-plans/issue_plan_0001",
      ],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    await waitFor(() => {
      expect(history.location.pathname).toBe("/workbench");
    });
    expect(history.location.search).toContain("overlay=plan-overview");
    expect(history.location.search).toContain("planId=issue_plan_0001");
    // replace 语义：不新增历史记录。
    expect(history.length).toBe(1);
    // 重定向组件自种地址簿——抽屉直接解析并渲染（取数失败即错误面证据）。
    expect(await screen.findByTestId("plan-overview-drawer")).toBeInTheDocument();
    expect(await screen.findByTestId("plan-overview-error")).toBeInTheDocument();
  });

  it("opens the plan overview drawer from a coding page and keeps one detail instance", async () => {
    loadAvailableProviders();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("plan aggregate not configured for this test");
      }),
    );
    const history = createMemoryHistory({
      initialEntries: [
        "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_web_0001?planId=issue_plan_0001",
      ],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    await userEvent.click(
      await screen.findByRole("button", { name: "返回" }),
    );

    await waitFor(() => {
      expect(history.location.pathname).toBe("/workbench");
    });
    expect(history.location.search).toContain("overlay=plan-overview");
    expect(history.location.search).toContain("planId=issue_plan_0001");
    expect(await screen.findByTestId("plan-overview-drawer")).toBeInTheDocument();
    // 离开 coding 路由后详情卸载（背景=Issue 工作台，禁止双挂载）。
    expect(screen.queryByTestId("coding-workspace-page")).toBeNull();
  });

  it("clears the overlay fields on drawer close while preserving focus", async () => {
    loadAvailableProviders();
    seedPlanAddress();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("plan aggregate not configured for this test");
      }),
    );
    const history = createMemoryHistory({
      initialEntries: [
        "/workbench?focus=issue%3Aissue_0001%3Aissue_0001&overlay=plan-overview&planId=issue_plan_0001",
      ],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("plan-overview-drawer")).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "关闭Coding Plan 总览" }),
    );

    await waitFor(() => {
      expect(screen.queryByTestId("plan-overview-drawer")).toBeNull();
    });
    expect(history.location.search).toContain(
      "focus=issue%3Aissue_0001%3Aissue_0001",
    );
    expect(history.location.search).not.toContain("overlay=");
    expect(history.location.search).not.toContain("planId=");
  });

  it("registers the inbox overlay skeleton for UI-3 and closes it via the shared pipeline", async () => {
    loadAvailableProviders();
    const history = createMemoryHistory({
      initialEntries: ["/workbench?overlay=inbox"],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("workbench-inbox-overlay")).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "关闭待处理事项" }),
    );

    await waitFor(() => {
      expect(screen.queryByTestId("workbench-inbox-overlay")).toBeNull();
    });
    expect(history.location.search).not.toContain("overlay=");
  });

  it("merges focus writes with the open overlay state instead of overwriting it", async () => {
    loadAvailableProviders();
    seedPlanAddress();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("plan aggregate not configured for this test");
      }),
    );
    const history = createMemoryHistory({
      initialEntries: [
        "/workbench?overlay=plan-overview&planId=issue_plan_0001",
      ],
    });

    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("plan-overview-drawer")).toBeInTheDocument();

    // 必改点回归：实体抽屉 focus 写入不得抹掉 overlay 定位字段。
    await userEvent.click(screen.getByRole("button", { name: "聚焦实体" }));

    await waitFor(() => {
      expect(history.location.search).toContain(
        "focus=issue%3Aissue_0001%3Aissue_0001",
      );
    });
    expect(history.location.search).toContain("overlay=plan-overview");
    expect(history.location.search).toContain("planId=issue_plan_0001");
    expect(screen.getByTestId("plan-overview-drawer")).toBeInTheDocument();
  });
});
