// web/src/components/coding-workspace/dashboard/CodingDashboard.test.tsx
import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CodingAttentionItemDto,
  CodingExecutionUnit,
  PlanCodingProgressDto,
  PlanCodingTargetDto,
  PlanCodingUnitProgressDto,
} from "../../../api/types";
import { useCodingWorkspaceStore } from "../../../state/coding-workspace-store";
import { CodingDashboard } from "./CodingDashboard";

const unit: CodingExecutionUnit = {
  unit_id: "coding_unit_0001",
  logical_work_item_id: "WI-001",
  work_item_revision_id: "r1",
  dependency_logical_work_item_ids: [],
  order_index: 0,
  status: "running",
  summary: "登录",
  latest_handoff_revision_id: null,
  completion_commit: null,
};

// owner 场景（issue_work_item_plan_0001）：busi 桶 session units 只有 WI-001，
// plan 全图 5 WI/4 仓；WI-002@api 依赖 busi 桶的 WI-001（跨桶等待）。
function planUnit(overrides: Partial<PlanCodingUnitProgressDto> = {}): PlanCodingUnitProgressDto {
  return {
    unit_id: "coding_unit_0001",
    logical_work_item_id: "WI-001",
    target_repository_id: "repo_busi",
    attempt_id: "coding_attempt_busi_0001",
    unit_status: "pending",
    execution_state: "waiting_slot",
    unsatisfied_dependency_ids: [],
    waiting_reason: null,
    dependency_logical_work_item_ids: [],
    order_index: 0,
    summary: null,
    ...overrides,
  };
}

function planTarget(repositoryId: string, attemptId: string): PlanCodingTargetDto {
  return {
    target_repository_id: repositoryId,
    attempt_id: attemptId,
    attempt_status: "created",
    worktree_path: null,
    lease_state: "absent",
    // FE-1：PlanCodingTargetDto 新增 S3 逐仓事实九字段；本 fixture 只覆盖
    // execution/lease 事实，逐仓交付事实一律 null。
    repository_name: null,
    branch_name: null,
    base_branch: null,
    stage: null,
    head_commit: null,
    push_status: null,
    push_error: null,
    review_request_id: null,
    manual_recovery_reason: null,
  };
}

function multiRepoPlanProgressDto(): PlanCodingProgressDto {
  return {
    plan_id: "issue_work_item_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: "plan_start_abc123",
    mode: "first_ready_unit",
    phase: "paused",
    primary_attempt_id: "coding_attempt_busi_0001",
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_busi_0001",
    target_attempts: [
      planTarget("repo_busi", "coding_attempt_busi_0001"),
      planTarget("repo_api", "coding_attempt_api_0001"),
      planTarget("repo_gateway", "coding_attempt_gateway_0001"),
      planTarget("repo_frontend", "coding_attempt_frontend_0001"),
    ],
    units: [
      planUnit({
        unit_id: "coding_unit_0001",
        logical_work_item_id: "WI-001",
        unit_status: "running",
        execution_state: "running",
        summary: "busi 首个单元",
      }),
      planUnit({
        unit_id: "coding_unit_0002",
        logical_work_item_id: "WI-002",
        target_repository_id: "repo_api",
        attempt_id: "coding_attempt_api_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["WI-001"],
        waiting_reason: "dependency_not_ready",
        dependency_logical_work_item_ids: ["WI-001"],
        order_index: 1,
      }),
      planUnit({
        unit_id: "coding_unit_0003",
        logical_work_item_id: "WI-003",
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        order_index: 2,
      }),
      planUnit({
        unit_id: "coding_unit_0004",
        logical_work_item_id: "WI-004",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        dependency_logical_work_item_ids: ["WI-003"],
        unsatisfied_dependency_ids: ["WI-003"],
        execution_state: "waiting_dependency",
        waiting_reason: "dependency_not_ready",
        order_index: 3,
      }),
      planUnit({
        unit_id: "coding_unit_0005",
        logical_work_item_id: "WI-005",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        dependency_logical_work_item_ids: ["WI-003", "WI-004"],
        unsatisfied_dependency_ids: ["WI-003", "WI-004"],
        execution_state: "waiting_dependency",
        waiting_reason: "dependency_not_ready",
        order_index: 4,
      }),
    ],
    plan_complete: false,
  };
}

function seedGroupAttemptStore(): void {
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_busi_0001",
    units: [unit],
    timelineNodes: [],
    currentWorkItemId: "WI-001",
    attemptScope: "work_item_group",
    workItemGroupId: "issue_work_item_plan_0001",
    status: "running",
    stage: "coding",
  });
}

function planAttentionItem(
  overrides: Partial<CodingAttentionItemDto> & { key: string },
): CodingAttentionItemDto {
  return {
    kind: "choice",
    project_id: "project_0001",
    issue_id: "issue_0001",
    title: "待处理",
    summary: "待处理事项",
    occurred_at: "2026-10-16T00:00:00Z",
    target: { type: "coding_attempt" },
    ...overrides,
  };
}

describe("CodingDashboard", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.setState({
      attemptId: null,
      units: [unit],
      timelineNodes: [],
      currentWorkItemId: "WI-001",
      attemptScope: null,
      workItemGroupId: null,
      status: null,
      stage: null,
    });
  });

  afterEach(() => {
    useCodingWorkspaceStore.setState({
      attemptId: null,
      units: [],
      timelineNodes: [],
      currentWorkItemId: null,
      attemptScope: null,
      workItemGroupId: null,
      status: null,
      stage: null,
    });
    vi.unstubAllGlobals();
  });

  // F-55 对比度：「只读视图」副标题小灰字提升到 slate-600，压浅灰底仍可读。
  it("renders the read-only caption in the higher-contrast slate-600 token", () => {
    render(<CodingDashboard />);
    const caption = screen.getByText("只读视图 · 动作入口统一收口于 Phase 4");
    expect(caption.className).toContain("text-slate-600");
  });

  it("does not fetch the plan aggregate outside a group attempt", () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    render(<CodingDashboard projectId="project_0001" issueId="issue_0001" />);

    expect(screen.getByTestId("coding-topology-graph")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("projects the full multi-repo plan graph with cross-bucket waiting cards", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(multiRepoPlanProgressDto()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);
    seedGroupAttemptStore();

    const { container } = render(
      <CodingDashboard projectId="project_0001" issueId="issue_0001" />,
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001/coding-start",
    );

    // 拓扑含全部 5 WI：跨桶 WI-002..WI-005 自动出现为等待灰卡（pending）。
    await waitFor(() => {
      expect(container.querySelector('[data-node="WI-005"]')).not.toBeNull();
    });
    for (const workItemId of ["WI-001", "WI-002", "WI-003", "WI-004", "WI-005"]) {
      const node = container.querySelector(`[data-node="${workItemId}"]`);
      expect(node, `node ${workItemId} missing`).not.toBeNull();
    }
    expect(container.querySelector('[data-node="WI-001"]')?.getAttribute("data-state")).toBe(
      "running",
    );
    expect(container.querySelector('[data-node="WI-002"]')?.getAttribute("data-state")).toBe(
      "pending",
    );
    // WI-001 的跨仓下游边自动出现（WI-001 未完成 → blocking）。
    const crossBucketEdge = container.querySelector('[data-edge="WI-001__WI-002"]');
    expect(crossBucketEdge).not.toBeNull();
    expect(crossBucketEdge?.getAttribute("data-edge-state")).toBe("blocking");

    // 依赖链跨仓语义：WI-001 的下游消费方包含跨桶 WI-002（不再是「无」）。
    const downstream = await screen.findByTestId("coding-chain-downstream");
    expect(downstream).toHaveTextContent("WI-002");
  });

  it("falls back to in-bucket session units when the plan aggregate is unavailable", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        Promise.reject(new Error("aggregate unavailable")),
    );
    vi.stubGlobal("fetch", fetchMock);
    seedGroupAttemptStore();

    const { container } = render(
      <CodingDashboard projectId="project_0001" issueId="issue_0001" />,
    );

    // 回落桶内口径：只有 busi 桶的 WI-001，无跨桶节点、无异常。
    await waitFor(() => {
      expect(container.querySelectorAll("[data-node]").length).toBe(1);
    });
    expect(container.querySelector('[data-node="WI-001"]')).not.toBeNull();
    expect(container.querySelector('[data-node="WI-002"]')).toBeNull();
    const downstream = screen.getByTestId("coding-chain-downstream");
    expect(downstream).toHaveTextContent("无下游消费");
  });

  it("keeps session units when the plan binds a single target attempt (v1)", async () => {
    const dto = multiRepoPlanProgressDto();
    dto.target_attempts = [dto.target_attempts[0]!];
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(dto), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);
    seedGroupAttemptStore();

    const { container } = render(
      <CodingDashboard projectId="project_0001" issueId="issue_0001" />,
    );

    await waitFor(() => {
      expect(container.querySelectorAll("[data-node]").length).toBe(1);
    });
    expect(container.querySelector('[data-node="WI-001"]')).not.toBeNull();
    expect(container.querySelector('[data-node="WI-002"]')).toBeNull();
  });

  // P1-d（REQ-MTG-13）：attempt 全态状态条——多仓取聚合 target_attempts；
  // WaitingForHuman＋FinalConfirm 显示「等待最终确认」不叫分诊；unit 已完成
  // 与 attempt 待确认可同时呈现（六态拓扑判据不变）。
  it("renders the aggregate attempt status strip with final-confirm wording beside the untouched topology", async () => {
    const dto = multiRepoPlanProgressDto();
    dto.units[0] = {
      ...dto.units[0]!,
      unit_status: "completed",
    };
    dto.target_attempts = [
      {
        ...planTarget("repo_busi", "coding_attempt_busi_0001"),
        attempt_status: "waiting_for_human",
        stage: "final_confirm",
        repository_name: "busi",
      },
      {
        ...planTarget("repo_api", "coding_attempt_api_0001"),
        attempt_status: "running",
        stage: "coding",
        repository_name: "api",
      },
      {
        ...planTarget("repo_gateway", "coding_attempt_gateway_0001"),
        attempt_status: "awaiting_plan_amendment",
        repository_name: "gateway",
      },
      {
        ...planTarget("repo_frontend", "coding_attempt_frontend_0001"),
        attempt_status: "amendment_apply_failed",
        repository_name: "frontend",
      },
    ];
    dto.attention_items = [
      planAttentionItem({
        key: "k_choice",
        kind: "choice",
        attempt_id: "coding_attempt_api_0001",
      }),
      planAttentionItem({
        key: "k_triage",
        kind: "triage",
        attempt_id: "coding_attempt_gateway_0001",
      }),
    ];
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(dto), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);
    seedGroupAttemptStore();

    const { container } = render(
      <CodingDashboard projectId="project_0001" issueId="issue_0001" />,
    );

    const strip = await screen.findByTestId("coding-dashboard-attempts");
    expect(
      screen.getByTestId("coding-dashboard-attempt-coding_attempt_busi_0001"),
    ).toHaveTextContent("等待最终确认");
    expect(strip).toHaveTextContent("busi");
    expect(strip).toHaveTextContent("api");
    expect(strip).toHaveTextContent("执行中 · 编码");
    expect(strip).toHaveTextContent("等待计划修订");
    expect(strip).toHaveTextContent("待办 choice");
    expect(strip).toHaveTextContent("待办 triage");
    expect(strip).not.toHaveTextContent("等待分诊");
    expect(strip).not.toHaveTextContent("等待人工");

    // unit 已完成与 attempt 待确认同时呈现；六态拓扑判据不改。
    await waitFor(() => {
      expect(container.querySelector('[data-node="WI-001"]')?.getAttribute("data-state")).toBe(
        "done",
      );
    });
  });

  it("renders the current session attempt outside a group attempt without fetching", () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "final_confirm",
    });

    render(<CodingDashboard projectId="project_0001" issueId="issue_0001" />);

    const strip = screen.getByTestId("coding-dashboard-attempts");
    expect(strip).toHaveTextContent("coding_attempt_0001");
    expect(strip).toHaveTextContent("等待最终确认");
    expect(strip).not.toHaveTextContent("等待分诊");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
