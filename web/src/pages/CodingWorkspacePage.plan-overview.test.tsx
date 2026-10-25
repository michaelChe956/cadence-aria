import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  RouterProvider,
} from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  PlanCodingProgressDto,
  ProviderHealthResponse,
} from "../api/types";
import { getPlanCodingProgress } from "../api/client";
import { createAppRouter } from "../router";
import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useWorkspaceSessionObservers } from "../hooks/useWorkspaceSessionObservers";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  DEFAULT_PERMISSION_MODES,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  readyCodingState,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  confirmWorkItemExecutionPlan: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  // 默认无 plan 聚合（旧用例降级回局部投影）；需要聚合的用例单独 mockResolvedValue。
  getPlanCodingProgress: vi.fn(async () => {
    throw new Error("plan aggregate not configured for this test");
  }),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

// P1-c 路由集成用例渲染 RootRouteComponent（CockpitShell）——observer 依赖
// 统一打桩（与 router.test/pending-choice 同款空投影）。
vi.mock("../hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: vi.fn(),
}));

// UI-1：coding 页返回总览落到 /workbench 抽屉（overlay=plan-overview）；
// AppShell（Issue 工作台）与本页断言无关，打桩防真实工作台挂载发请求。
vi.mock("../app-shell", () => ({
  AppShell: () => <div data-testid="workbench-page">Workbench</div>,
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({
    value,
    language,
    height,
  }: {
    value: string;
    language?: string;
    height?: string;
  }) => (
    <div data-testid="monaco-viewer" data-language={language} data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({
    original,
    modified,
    language,
    height,
  }: {
    original: string;
    modified: string;
    language?: string;
    height?: string;
  }) => (
    <div data-testid="monaco-diff-viewer" data-language={language} data-height={height}>
      <span data-testid="monaco-diff-original">{original}</span>
      <span data-testid="monaco-diff-modified">{modified}</span>
    </div>
  ),
}));

// P1-c：定位断言共用——记录每个 scrollIntoView 调用落点的元素。
function spyScrollIntoView() {
  const scrolled: HTMLElement[] = [];
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value(this: HTMLElement) {
      scrolled.push(this);
    },
  });
  return scrolled;
}

describe("P1-c router integration", () => {
  installCodingWorkspacePageTestHooks();

  beforeEach(() => {
    vi.mocked(useWorkspaceSessionObservers).mockReturnValue({
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


  function availableProviderHealth(): ProviderHealthResponse {
    return {
      schema_version: 1,
      generation: 1,
      checked_at: "2026-10-16T00:00:00Z",
      state_status: "ready",
      state_error: null,
      real_workflow_blocked: false,
      test_provider_enabled: false,
      providers: [],
    };
  }

  function loadProviderAvailability() {
    const health = availableProviderHealth();
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

  // P1-c 红测①②：带 planId/focus search 的 coding URL 落到正确 attempt 地址
  // （WS 以该地址重置/重连），final_confirm 焦点定位确认区；红测④：返回
  // 按钮回到 plan 总览路由（planId 上下文）。
  it("routes a focused coding URL and returns to the plan overview", async () => {
    const scrolled = spyScrollIntoView();
    loadProviderAvailability();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      attemptScope: "work_item_group",
      status: "waiting_for_human",
      stage: "final_confirm",
    });

    const history = createMemoryHistory({
      initialEntries: [
        "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001" +
          "?planId=work_item_plan_0001&workItemId=work_item_0001&focus=final_confirm",
      ],
    });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    expect(vi.mocked(useCodingWorkspaceWs)).toHaveBeenCalledWith(
      expect.objectContaining({ attemptId: "coding_attempt_0001" }),
    );
    const banner = await screen.findByTestId("coding-final-confirm-waiting");
    expect(scrolled).toContain(banner);

    await userEvent.click(screen.getByRole("button", { name: "返回" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe("/workbench");
    });
    expect(history.location.search).toContain("overlay=plan-overview");
    expect(history.location.search).toContain("planId=work_item_plan_0001");
    // coding 页入口在导航前种入 plan 地址簿——抽屉开箱即解析；
    // getPlanCodingProgress 默认 mock 抛错，总览错误面即「已回到总览」证据。
    expect(await screen.findByTestId("plan-overview-drawer")).toBeInTheDocument();
    expect(await screen.findByTestId("plan-overview-error")).toBeInTheDocument();
    expect(screen.queryByTestId("coding-workspace-page")).toBeNull();
  });
});

// ── F-B（owner 真机复验 2026-10-25）：「查看全部 WI」改为 Coding Attempt 页
// 内的弹窗——不跨页导航、不跳主页抽屉、关闭即停轮询、明细不双挂载 ──

describe("F-B plan overview in-page modal", () => {
  installCodingWorkspacePageTestHooks();

  beforeEach(() => {
    vi.mocked(useWorkspaceSessionObservers).mockReturnValue({
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

  function availableProviderHealth(): ProviderHealthResponse {
    return {
      schema_version: 1,
      generation: 1,
      checked_at: "2026-10-16T00:00:00Z",
      state_status: "ready",
      state_error: null,
      real_workflow_blocked: false,
      test_provider_enabled: false,
      providers: [],
    };
  }

  function loadProviderAvailability() {
    const health = availableProviderHealth();
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

  function planProgressDto(): PlanCodingProgressDto {
    return {
      plan_id: "work_item_plan_0001",
      bound_plan_revision_id: "plan_revision_0001",
      command_id: "plan_start_0001",
      mode: "first_ready_unit",
      phase: "running",
      primary_attempt_id: "coding_attempt_0001",
      selected_unit_id: "coding_unit_0001",
      selected_attempt_id: "coding_attempt_0001",
      target_attempts: [
        {
          target_repository_id: "repo_0001",
          attempt_id: "coding_attempt_0001",
          attempt_status: "running",
          worktree_path: "/tmp/wt",
          lease_state: "held",
          repository_name: "主仓",
          branch_name: "feat/1",
          base_branch: "main",
          stage: "coding",
          head_commit: null,
          push_status: null,
          push_error: null,
          review_request_id: null,
          manual_recovery_reason: null,
        },
        {
          target_repository_id: "repo_0002",
          attempt_id: "coding_attempt_web_0001",
          attempt_status: "running",
          worktree_path: null,
          lease_state: "held",
          repository_name: "web 仓",
          branch_name: null,
          base_branch: null,
          stage: "coding",
          head_commit: null,
          push_status: null,
          push_error: null,
          review_request_id: null,
          manual_recovery_reason: null,
        },
      ],
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          target_repository_id: "repo_0001",
          attempt_id: "coding_attempt_0001",
          unit_status: "running",
          execution_state: "running",
          unsatisfied_dependency_ids: [],
          waiting_reason: null,
          dependency_logical_work_item_ids: [],
          order_index: 0,
          summary: "主仓工作项",
        },
        {
          unit_id: "coding_unit_0002",
          logical_work_item_id: "work_item_0002",
          target_repository_id: "repo_0002",
          attempt_id: "coding_attempt_web_0001",
          unit_status: "running",
          execution_state: "running",
          unsatisfied_dependency_ids: [],
          waiting_reason: null,
          dependency_logical_work_item_ids: [],
          order_index: 1,
          summary: null,
        },
      ],
      final_confirm: {
        state: "not_ready",
        ready_attempt_ids: [],
        confirmed_attempt_ids: [],
        blocking: [],
      },
      attention_items: [],
      plan_complete: false,
    };
  }

  function groupSessionState() {
    useCodingWorkspaceStore.getState().setSessionState({
      type: "coding_session_state",
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: "coding_attempt_0001",
      attempt_scope: "work_item_group",
      work_item_group_id: "work_item_plan_0001",
      current_work_item_id: "work_item_0001",
      active_unit_id: "coding_unit_0001",
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          work_item_revision_id: "work_item_revision_0001",
          dependency_logical_work_item_ids: [],
          order_index: 0,
          status: "running",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
      ],
      status: "running",
      stage: "coding",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      rework_count: 0,
      max_auto_rework: 2,
      head_commit: null,
      pushed_remote: null,
      provider_config_snapshot: {
        author: "fake",
        reviewer: "fake",
        review_rounds: 1,
      },
      role_provider_config_snapshot: {
        coder: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: { ...DEFAULT_PERMISSION_MODES },
      },
      timeline_nodes: [],
      active_node_id: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
      pending_choices: [],
      role_runs: [],
      chat_entries: [],
      work_item_markdown: null,
      verification_commands: [],
      work_item_execution_plan: null,
      linked_plan_repair: null,
      require_execution_plan_confirm: false,
    });
  }

  const CODING_PATH =
    "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001";

  // F-B 核心：入口在同页弹窗里开总览，URL 不变、不跳 /workbench。
  it("opens the overview in an in-page modal without navigating away", async () => {
    loadProviderAvailability();
    mockCodingWs();
    groupSessionState();
    vi.mocked(getPlanCodingProgress).mockResolvedValue(planProgressDto());

    const history = createMemoryHistory({ initialEntries: [CODING_PATH] });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    await userEvent.click(await screen.findByTestId("plan-coding-open-overview"));

    expect(await screen.findByTestId("plan-overview-modal")).toBeInTheDocument();
    expect(await screen.findByTestId("plan-overview-units")).toBeInTheDocument();
    // 同页：URL 未变、没有跨页导航（router 侧主页抽屉没被打开）。
    expect(history.location.pathname).toBe(CODING_PATH);
    expect(history.location.search).not.toContain("overlay=");
    expect(screen.queryByTestId("plan-overview-drawer")).toBeNull();
    // 没有第二个 attempt 实例：仍是同一个 coding 页。
    expect(screen.getAllByTestId("coding-workspace-page")).toHaveLength(1);
  });

  // F-B：关闭弹窗即停轮询（弹窗卸载 → 5 秒轮询句柄释放）。
  it("stops the plan polling when the modal closes", async () => {
    vi.useFakeTimers();
    mockCodingWs();
    groupSessionState();
    const progressMock = vi.mocked(getPlanCodingProgress);
    progressMock.mockResolvedValue(planProgressDto());

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    fireEvent.click(screen.getByTestId("plan-coding-open-overview"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const opened = progressMock.mock.calls.length;

    // 弹窗可见期间按 5 秒轮询。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(progressMock.mock.calls.length).toBeGreaterThan(opened);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("plan-overview-modal")).toBeNull();
    const closed = progressMock.mock.calls.length;

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(progressMock.mock.calls.length).toBe(closed);
    vi.useRealTimers();
  });

  // F-B：明细下钻只导航一次到目标 attempt，不双挂载第二个 attempt 实例。
  it("navigates to the selected attempt without mounting a second instance", async () => {
    loadProviderAvailability();
    mockCodingWs();
    groupSessionState();
    vi.mocked(getPlanCodingProgress).mockResolvedValue(planProgressDto());

    const history = createMemoryHistory({ initialEntries: [CODING_PATH] });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    await userEvent.click(await screen.findByTestId("plan-coding-open-overview"));
    await screen.findByTestId("plan-overview-units");
    await userEvent.click(screen.getByTestId("plan-overview-open-work_item_0002"));

    await waitFor(() => {
      expect(history.location.pathname).toBe(
        "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_web_0001",
      );
    });
    // 弹窗随下钻关闭（详情不在弹窗里就地展开）。
    expect(screen.queryByTestId("plan-overview-modal")).toBeNull();
    // 单实例：任一时刻只有一个 attempt 详情页挂载。
    expect(screen.getAllByTestId("coding-workspace-page")).toHaveLength(1);
  });

  // F-B（owner 复验残项）：「返回」按来源分流——A 类（同页弹窗下钻）回来源
  // attempt 页并自动重开总览弹窗，不落主页抽屉。
  it("returns a modal drilldown to the source attempt with the overview reopened", async () => {
    loadProviderAvailability();
    mockCodingWs();
    groupSessionState();
    vi.mocked(getPlanCodingProgress).mockResolvedValue(planProgressDto());

    const history = createMemoryHistory({ initialEntries: [CODING_PATH] });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    await userEvent.click(await screen.findByTestId("plan-coding-open-overview"));
    await screen.findByTestId("plan-overview-units");
    await userEvent.click(screen.getByTestId("plan-overview-open-work_item_0002"));

    const DRILLED_PATH =
      "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_web_0001";
    await waitFor(() => {
      expect(history.location.pathname).toBe(DRILLED_PATH);
    });
    // 下钻地址带上来源标记与来源 attempt（A 类返回落点依据）。
    expect(history.location.search).toContain("from=plan-overview-modal");
    expect(history.location.search).toContain(
      "backAttemptId=coding_attempt_0001",
    );

    await userEvent.click(await screen.findByRole("button", { name: "返回" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe(CODING_PATH);
    });
    expect(history.location.search).toContain("openOverview=true");
    // 不落主页抽屉：仍是 coding 页形态，总览弹窗自动重开。
    expect(screen.queryByTestId("plan-overview-drawer")).toBeNull();
    expect(await screen.findByTestId("plan-overview-modal")).toBeInTheDocument();
    expect(await screen.findByTestId("plan-overview-units")).toBeInTheDocument();
    expect(screen.getAllByTestId("coding-workspace-page")).toHaveLength(1);

    // 用户关闭后不再被 openOverview 请求拉回（弹窗开关归用户；URL 请求值
    // 只在换 attempt 时重放）。
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("plan-overview-modal")).toBeNull();
    expect(history.location.search).toContain("openOverview=true");
  });

  // F-B：「返回」B 类——工作台抽屉（总览）来源保持既有抽屉形态，不回主页。
  it("returns an overlay-sourced attempt to the plan overview drawer", async () => {
    loadProviderAvailability();
    mockCodingWs();
    groupSessionState();
    vi.mocked(getPlanCodingProgress).mockResolvedValue(planProgressDto());

    const history = createMemoryHistory({
      initialEntries: [
        "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_web_0001" +
          "?planId=work_item_plan_0001&from=workbench-overlay",
      ],
    });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "返回" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe("/workbench");
    });
    expect(history.location.search).toContain("overlay=plan-overview");
    expect(history.location.search).toContain("planId=work_item_plan_0001");
    expect(await screen.findByTestId("plan-overview-drawer")).toBeInTheDocument();
    // 抽屉形态不双挂载：coding 页随路由离开卸载。
    expect(screen.queryByTestId("coding-workspace-page")).toBeNull();
  });

  // F-B：「返回」C 类——无来源（直接打开 attempt 页）回工作台主页且不带
  // overlay（不再无脑落主页抽屉）。
  it("returns a source-less attempt to the bare workbench root", async () => {
    loadProviderAvailability();
    mockCodingWs();
    groupSessionState();

    const history = createMemoryHistory({ initialEntries: [CODING_PATH] });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "返回" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe("/workbench");
    });
    expect(history.location.search).not.toContain("overlay=");
    expect(history.location.search).not.toContain("planId=");
    expect(screen.queryByTestId("plan-overview-drawer")).toBeNull();
  });
});
