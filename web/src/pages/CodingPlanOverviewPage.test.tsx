import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import type { PlanCodingProgressDto } from "../api/types";
import { CodingPlanOverviewView } from "./CodingPlanOverviewPage";
import {
  CodingPlanOverviewDrawer,
} from "./CodingPlanOverviewDrawer";
import type { PlanOverviewOpenTarget } from "./CodingPlanOverviewPage";
import { useWorkbenchOverlayDirectory } from "../state/workbench-overlay-store";

// UI-1：抽屉内详情复用既有 CodingWorkspacePage——测试用轻桩替身（真实页
// 由 CodingWorkspacePage.test.tsx 覆盖），只断言地址/焦点/回调接线。
vi.mock("./CodingWorkspacePage", () => ({
  CodingWorkspacePage: ({
    address,
    focus,
    onBack,
  }: {
    address: { projectId: string; issueId: string; attemptId: string };
    focus?: { gateId?: string; choiceId?: string };
    onBack: () => void;
  }) => (
    <div
      data-testid="coding-workspace-page-stub"
      data-project-id={address.projectId}
      data-issue-id={address.issueId}
      data-attempt-id={address.attemptId}
      data-gate-id={focus?.gateId ?? ""}
    >
      <button type="button" onClick={onBack}>
        返回
      </button>
    </div>
  ),
}));

// P1-b（REQ-MTG-12）：只读 plan 总览——列全 WI/仓库/unit 进度/attempt 状态/
// 依赖/待办徽标；标题执行/确认 N/M；5 秒轮询见非当前仓更新；聚合确认一次 POST；
// 除 plan 聚合确认外无业务写操作。夹具镜像后端 PlanCodingProgressDto（含
// P1-a final_confirm 与 P4-c attention_items 投影）。
function overviewDto(
  overrides: Partial<PlanCodingProgressDto> = {},
): PlanCodingProgressDto {
  return {
    plan_id: "issue_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: "plan_start_abc123",
    mode: "first_ready_unit",
    phase: "running",
    primary_attempt_id: "coding_attempt_busi_0001",
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_busi_0001",
    target_attempts: [
      {
        target_repository_id: "repo_busi",
        attempt_id: "coding_attempt_busi_0001",
        attempt_status: "waiting_for_human",
        worktree_path: "/tmp/wt/busi",
        lease_state: "held",
        repository_name: "busi 服务仓",
        branch_name: "feat/busi-1",
        base_branch: "main",
        stage: "final_confirm",
        head_commit: null,
        push_status: null,
        push_error: null,
        review_request_id: null,
        manual_recovery_reason: null,
      },
      {
        target_repository_id: "repo_web",
        attempt_id: "coding_attempt_web_0001",
        attempt_status: "running",
        worktree_path: null,
        lease_state: "held",
        repository_name: "web 前端仓",
        branch_name: null,
        base_branch: null,
        stage: "coding",
        head_commit: null,
        push_status: null,
        push_error: null,
        review_request_id: null,
        manual_recovery_reason: null,
      },
      {
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
        repository_name: null,
        branch_name: null,
        base_branch: null,
        stage: null,
        head_commit: null,
        push_status: null,
        push_error: null,
        review_request_id: null,
        manual_recovery_reason: null,
      },
      {
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
        repository_name: null,
        branch_name: null,
        base_branch: null,
        stage: null,
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
        logical_work_item_id: "wi_core",
        target_repository_id: "repo_busi",
        attempt_id: "coding_attempt_busi_0001",
        unit_status: "completed",
        execution_state: "completed",
        unsatisfied_dependency_ids: [],
        waiting_reason: null,
        dependency_logical_work_item_ids: [],
        order_index: 0,
        summary: "核心域订单模型",
      },
      {
        unit_id: "coding_unit_0002",
        logical_work_item_id: "wi_registration",
        target_repository_id: "repo_web",
        attempt_id: "coding_attempt_web_0001",
        unit_status: "running",
        execution_state: "running",
        unsatisfied_dependency_ids: [],
        waiting_reason: null,
        dependency_logical_work_item_ids: ["wi_core"],
        order_index: 1,
        summary: null,
      },
      {
        unit_id: "coding_unit_0003",
        logical_work_item_id: "wi_unrelated",
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        unit_status: "pending",
        execution_state: "waiting_slot",
        unsatisfied_dependency_ids: [],
        waiting_reason: "execution_slot_occupied",
        dependency_logical_work_item_ids: [],
        order_index: 2,
        summary: "网关旁路",
      },
      {
        unit_id: "coding_unit_0004",
        logical_work_item_id: "wi_service",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["wi_core"],
        waiting_reason: "dependency_not_ready",
        dependency_logical_work_item_ids: ["wi_core"],
        order_index: 3,
        summary: null,
      },
      {
        unit_id: "coding_unit_0005",
        logical_work_item_id: "wi_panel",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        unit_status: "pending",
        execution_state: "initialized",
        unsatisfied_dependency_ids: ["wi_service"],
        waiting_reason: "dependency_not_ready",
        dependency_logical_work_item_ids: ["wi_service"],
        order_index: 4,
        summary: null,
      },
    ],
    final_confirm: {
      state: "not_ready",
      ready_attempt_ids: [],
      confirmed_attempt_ids: [],
      blocking: [],
    },
    attention_items: [
      {
        key: "gate-busi-1",
        kind: "triage",
        project_id: "project_0001",
        issue_id: "issue_0001",
        plan_id: "issue_plan_0001",
        attempt_id: "coding_attempt_busi_0001",
        logical_work_item_id: "wi_core",
        repository_name: "busi 服务仓",
        reason_code: null,
        title: "分诊待处理",
        summary: "coder provider 失败等待分诊",
        occurred_at: "2026-10-16T10:00:00Z",
        target: { type: "coding_attempt", attempt_id: "coding_attempt_busi_0001" },
        focus: { gate_id: "gate_busi_1" },
      },
      {
        key: "choice-web-1",
        kind: "choice",
        project_id: "project_0001",
        issue_id: "issue_0001",
        plan_id: "issue_plan_0001",
        attempt_id: "coding_attempt_web_0001",
        logical_work_item_id: "wi_registration",
        repository_name: "web 前端仓",
        reason_code: null,
        title: "选择题待作答",
        summary: "注册流程二选一",
        occurred_at: "2026-10-16T10:05:00Z",
        target: { type: "coding_attempt", attempt_id: "coding_attempt_web_0001" },
        focus: { choice_id: "choice_web_1" },
      },
    ],
    plan_complete: false,
    ...overrides,
  };
}

const ADDRESS = {
  projectId: "project_0001",
  issueId: "issue_0001",
  planId: "issue_plan_0001",
} as const;

const OVERVIEW_GET_URL =
  "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-start";
const FINAL_CONFIRM_POST_URL =
  "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-final-confirm";

function renderOverview(
  dto: PlanCodingProgressDto = overviewDto(),
  onOpenAttempt = vi.fn(),
) {
  return render(
    <CodingPlanOverviewView
      {...ADDRESS}
      onOpenAttempt={onOpenAttempt}
      pollIntervalMs={5_000}
    />,
  );
}

describe("CodingPlanOverviewPage read-only overview (P1-b/REQ-MTG-12)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("lists every WI in order_index with repo, progress, attempt state, deps and waiting copy", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );

    renderOverview();

    const list = await screen.findByTestId("plan-overview-units");
    const rows = list.querySelectorAll("li");
    expect(rows).toHaveLength(5);
    expect(
      Array.from(rows).map((row) => row.getAttribute("data-testid")),
    ).toEqual([
      "plan-overview-unit-wi_core",
      "plan-overview-unit-wi_registration",
      "plan-overview-unit-wi_unrelated",
      "plan-overview-unit-wi_service",
      "plan-overview-unit-wi_panel",
    ]);

    const core = screen.getByTestId("plan-overview-unit-wi_core");
    expect(core).toHaveTextContent("wi_core");
    expect(core).toHaveTextContent("核心域订单模型"); // summary 透传
    expect(core).toHaveTextContent("busi 服务仓"); // repository_name 优先
    expect(core).toHaveTextContent("已完成"); // execution_state 文案
    expect(core).toHaveTextContent("等待人工"); // attempt 状态标签
    expect(core).toHaveTextContent("最终确认"); // stage 标签

    const registration = screen.getByTestId("plan-overview-unit-wi_registration");
    expect(registration).toHaveTextContent("wi_registration"); // summary 缺省回落 WI ID
    expect(registration).toHaveTextContent("web 前端仓");
    expect(registration).toHaveTextContent("provider 运行中");

    const unrelated = screen.getByTestId("plan-overview-unit-wi_unrelated");
    expect(unrelated).toHaveTextContent("@repo_gateway"); // repository_name 缺席回落 ID
    expect(unrelated).toHaveTextContent("等待执行槽位"); // waiting_reason 文案

    const service = screen.getByTestId("plan-overview-unit-wi_service");
    expect(service).toHaveTextContent("依赖 wi_core");
    expect(service).toHaveTextContent("等待依赖完成");
  });

  it("shows execution/confirmation N/M and separates plan_complete from human confirmation copy", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify(
              overviewDto({
                plan_complete: true,
                final_confirm: {
                  state: "done",
                  ready_attempt_ids: [],
                  confirmed_attempt_ids: [
                    "coding_attempt_busi_0001",
                    "coding_attempt_web_0001",
                    "coding_attempt_gateway_0001",
                    "coding_attempt_frontend_0001",
                  ],
                  blocking: [],
                },
              }),
            ),
            { status: 200 },
          ),
      ),
    );

    renderOverview();

    const stats = await screen.findByTestId("plan-overview-stats");
    expect(stats).toHaveTextContent("执行 1/5");
    expect(stats).toHaveTextContent("全部 WI 执行完成");
    expect(stats).toHaveTextContent("人工确认 4/4");
    expect(stats).toHaveTextContent("全部人工确认");
  });

  it("does not claim human completion when only plan execution is complete", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify(
              overviewDto({
                plan_complete: true,
                final_confirm: {
                  state: "ready",
                  ready_attempt_ids: [
                    "coding_attempt_busi_0001",
                    "coding_attempt_web_0001",
                    "coding_attempt_gateway_0001",
                    "coding_attempt_frontend_0001",
                  ],
                  confirmed_attempt_ids: [],
                  blocking: [],
                },
              }),
            ),
            { status: 200 },
          ),
      ),
    );

    renderOverview();

    const stats = await screen.findByTestId("plan-overview-stats");
    expect(stats).toHaveTextContent("全部 WI 执行完成");
    expect(stats).not.toHaveTextContent("全部人工确认");
    expect(stats).toHaveTextContent("人工确认 0/4");
  });

  it("opens a single attempt detail from the WI row via the existing attempt address", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );
    const onOpenAttempt = vi.fn();

    renderOverview(overviewDto(), onOpenAttempt);

    await screen.findByTestId("plan-overview-units");
    fireEvent.click(
      screen.getByTestId("plan-overview-open-wi_registration"),
    );

    expect(onOpenAttempt).toHaveBeenCalledTimes(1);
    expect(onOpenAttempt).toHaveBeenCalledWith({
      attemptId: "coding_attempt_web_0001",
      workItemId: "wi_registration",
    });
  });

  it("routes attention badges to the owning attempt with gate/choice focus", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );
    const onOpenAttempt = vi.fn();

    renderOverview(overviewDto(), onOpenAttempt);

    await screen.findByTestId("plan-overview-units");
    fireEvent.click(screen.getByTestId("plan-overview-attention-gate-busi-1"));
    expect(onOpenAttempt).toHaveBeenCalledWith({
      attemptId: "coding_attempt_busi_0001",
      workItemId: "wi_core",
      gateId: "gate_busi_1",
    });

    fireEvent.click(screen.getByTestId("plan-overview-attention-choice-web-1"));
    expect(onOpenAttempt).toHaveBeenCalledWith({
      attemptId: "coding_attempt_web_0001",
      workItemId: "wi_registration",
      choiceId: "choice_web_1",
    });
  });

  it("polls every 5s and reflects a non-current repo status change without manual refresh", async () => {
    vi.useFakeTimers();
    let call = 0;
    const fetchMock = vi.fn(async () => {
      call += 1;
      const dto =
        call === 1
          ? overviewDto()
          : overviewDto({
              units: overviewDto().units.map((unit) =>
                unit.logical_work_item_id === "wi_unrelated"
                  ? {
                      ...unit,
                      unit_status: "running",
                      execution_state: "running" as const,
                      waiting_reason: null,
                    }
                  : unit,
              ),
              target_attempts: overviewDto().target_attempts.map((target) =>
                target.target_repository_id === "repo_gateway"
                  ? { ...target, attempt_status: "running" as const }
                  : target,
              ),
            });
      return new Response(JSON.stringify(dto), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);

    renderOverview();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("plan-overview-unit-wi_unrelated")).toHaveTextContent(
      "已创建",
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(
      screen.getByTestId("plan-overview-unit-wi_unrelated"),
    ).toHaveTextContent("执行中");
  });

  it("shows explicit loading then error with a working retry", async () => {
    let rejectNext = true;
    const fetchMock = vi.fn(async () => {
      if (rejectNext) {
        rejectNext = false;
        return new Response(JSON.stringify({ code: "x", message: "no run" }), {
          status: 404,
        });
      }
      return new Response(JSON.stringify(overviewDto()), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);

    render(<CodingPlanOverviewView {...ADDRESS} onOpenAttempt={vi.fn()} />);
    expect(screen.getByTestId("plan-overview-loading")).toBeInTheDocument();

    const error = await screen.findByTestId("plan-overview-error");
    expect(error).toHaveTextContent("plan 聚合进度获取失败");
    expect(screen.queryByTestId("plan-overview-units")).toBeNull();

    fireEvent.click(screen.getByTestId("plan-overview-retry"));
    await screen.findByTestId("plan-overview-units");
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("confirms the whole plan with exactly one POST when final_confirm is ready", async () => {
    const fetchMock = vi.fn(
      async (input: RequestInfo | URL, _init?: RequestInit) => {
      if (String(input).endsWith("/coding-final-confirm")) {
        return new Response(
          JSON.stringify({
            plan_id: "issue_plan_0001",
            outcome: "all_confirmed",
            results: overviewDto().target_attempts.map((target) => ({
              attempt_id: target.attempt_id,
              target_repository_id: target.target_repository_id,
              status: "confirmed",
            })),
          }),
          { status: 200 },
        );
      }
      return new Response(
        JSON.stringify(
          overviewDto({
            final_confirm: {
              state: "ready",
              ready_attempt_ids: [
                "coding_attempt_busi_0001",
                "coding_attempt_web_0001",
                "coding_attempt_gateway_0001",
                "coding_attempt_frontend_0001",
              ],
              confirmed_attempt_ids: [],
              blocking: [],
            },
          }),
        ),
        { status: 200 },
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    renderOverview();

    const button = await screen.findByTestId("plan-final-confirm-button");
    expect(button).toHaveTextContent("确认本计划全部完成（4 仓/5 项）");
    fireEvent.click(button);

    await waitFor(() =>
      expect(screen.getByTestId("plan-final-confirm-result")).toBeInTheDocument(),
    );
    const posts = fetchMock.mock.calls.filter(
      ([input]) => String(input) === FINAL_CONFIRM_POST_URL,
    );
    expect(posts).toHaveLength(1);
    expect((posts[0]?.[1] as RequestInit | undefined)?.method).toBe("POST");
    expect((posts[0]?.[1] as RequestInit | undefined)?.body).toBeUndefined();
    // onConfirmed 之后总览重拉 GET 刷新真实状态（GET 次数 ≥ POST 次数 + 初始）。
    const gets = fetchMock.mock.calls.filter(
      ([input]) => String(input) === OVERVIEW_GET_URL,
    );
    expect(gets.length).toBeGreaterThanOrEqual(2);
  });

  it("hides the aggregate confirm button while final_confirm is not ready", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );

    renderOverview();

    await screen.findByTestId("plan-overview-units");
    expect(screen.queryByTestId("plan-final-confirm-button")).toBeNull();
  });
});

// ── UI-1（owner 拍板）：总览抽屉化——总览 ↔ 单个 Attempt 详情切换 ──

function DrawerHarness({
  planId = ADDRESS.planId,
  onOpenAttempt,
  onBackToOverview,
  onClose,
}: {
  planId?: string;
  onOpenAttempt: (target: PlanOverviewOpenTarget) => void;
  onBackToOverview: () => void;
  onClose: () => void;
}) {
  const [attempt, setAttempt] = useState<PlanOverviewOpenTarget | null>(null);
  return (
    <CodingPlanOverviewDrawer
      planId={planId}
      attemptId={attempt?.attemptId}
      workItemId={attempt?.workItemId}
      gateId={attempt?.gateId}
      choiceId={attempt?.choiceId}
      onOpenAttempt={(target) => {
        onOpenAttempt(target);
        setAttempt(target);
      }}
      onBackToOverview={() => {
        onBackToOverview();
        setAttempt(null);
      }}
      onClose={onClose}
    />
  );
}

function renderDrawer(
  handlers: Partial<{
    onOpenAttempt: (target: PlanOverviewOpenTarget) => void;
    onBackToOverview: () => void;
    onClose: () => void;
  }> = {},
  planId?: string,
) {
  return render(
    <DrawerHarness
      planId={planId}
      onOpenAttempt={handlers.onOpenAttempt ?? vi.fn()}
      onBackToOverview={handlers.onBackToOverview ?? vi.fn()}
      onClose={handlers.onClose ?? vi.fn()}
    />,
  );
}

describe("CodingPlanOverviewDrawer overview ↔ detail switching (UI-1)", () => {
  beforeEach(() => {
    useWorkbenchOverlayDirectory.getState().resetWorkbenchOverlayDirectory();
    useWorkbenchOverlayDirectory
      .getState()
      .seedWorkbenchOverlayAddress(ADDRESS.planId, {
        projectId: ADDRESS.projectId,
        issueId: ADDRESS.issueId,
      });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("renders the embedded overview inside the drawer and opens detail via a WI row", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );

    renderDrawer();

    expect(screen.getByTestId("plan-overview-drawer")).toBeInTheDocument();
    await screen.findByTestId("plan-overview-units");
    // 归档勿遗漏：聚合确认（PlanCodingFinalConfirm）随总览一并进抽屉。
    expect(screen.getByTestId("plan-overview-stats")).toBeInTheDocument();
    expect(screen.queryByTestId("coding-workspace-page-stub")).toBeNull();

    fireEvent.click(screen.getByTestId("plan-overview-open-wi_registration"));

    const detail = await screen.findByTestId("coding-workspace-page-stub");
    expect(detail).toHaveAttribute("data-project-id", ADDRESS.projectId);
    expect(detail).toHaveAttribute("data-issue-id", ADDRESS.issueId);
    expect(detail).toHaveAttribute("data-attempt-id", "coding_attempt_web_0001");
    // 禁止双挂载：任一时刻只有一个既有 Attempt 详情实例。
    expect(screen.queryAllByTestId("coding-workspace-page-stub")).toHaveLength(1);
  });

  it("routes attention badges into the drawer detail with gate focus", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );

    renderDrawer();

    await screen.findByTestId("plan-overview-units");
    fireEvent.click(screen.getByTestId("plan-overview-attention-gate-busi-1"));

    const detail = await screen.findByTestId("coding-workspace-page-stub");
    expect(detail).toHaveAttribute("data-attempt-id", "coding_attempt_busi_0001");
    expect(detail).toHaveAttribute("data-gate-id", "gate_busi_1");
  });

  it("returns to the overview with the scroll position restored", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(overviewDto()), { status: 200 })),
    );

    renderDrawer();

    await screen.findByTestId("plan-overview-units");
    const overviewBody = screen.getByTestId("plan-overview-drawer-overview");
    overviewBody.scrollTop = 240;
    fireEvent.scroll(overviewBody);

    fireEvent.click(screen.getByTestId("plan-overview-open-wi_registration"));
    await screen.findByTestId("coding-workspace-page-stub");
    // 总览保持挂载但隐藏（数据与滚动容器不重建）。
    expect(screen.getByTestId("plan-overview-drawer-overview").className).toContain(
      "hidden",
    );

    fireEvent.click(screen.getByRole("button", { name: "返回" }));

    expect(screen.queryByTestId("coding-workspace-page-stub")).toBeNull();
    expect(screen.getByTestId("plan-overview-units")).toBeVisible();
    expect(screen.getByTestId("plan-overview-drawer-overview").scrollTop).toBe(240);
  });

  it("stops the 5s polling as soon as the drawer closes", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn(
      async () => new Response(JSON.stringify(overviewDto()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const view = renderDrawer();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);

    view.unmount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("pauses the overview polling while a detail is mounted", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn(
      async () => new Response(JSON.stringify(overviewDto()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    renderDrawer();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId("plan-overview-open-wi_registration"));
    // fake timers 下不用 findBy（其轮询依赖真实定时器）——fireEvent 已在
    // act 内同步完成挂载。
    expect(
      screen.getByTestId("coding-workspace-page-stub"),
    ).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("shows a placeholder while the plan address is unresolved", () => {
    renderDrawer(undefined, "plan_without_seed");

    expect(
      screen.getByTestId("plan-overview-drawer-unresolved"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("plan-overview-view")).toBeNull();
  });
});
