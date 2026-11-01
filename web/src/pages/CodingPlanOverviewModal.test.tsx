import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import type { PlanCodingProgressDto } from "../api/types";
import {
  CodingPlanOverviewModal,
  type PlanOverviewOpenTarget,
} from "./CodingPlanOverviewModal";

// F-B（owner 真机复验 2026-10-25）：「查看全部 WI」总览改为 Coding Attempt
// 页内弹窗——本页承载、不跨页导航、关闭即停轮询、明细不双挂载。
// 夹具镜像后端 PlanCodingProgressDto（两仓两 WI，够验列表/下钻/轮询）。

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
    ],
    final_confirm: {
      state: "not_ready",
      ready_attempt_ids: [],
      confirmed_attempt_ids: [],
      blocking: [],
    },
    attention_items: [],
    plan_complete: false,
    ...overrides,
  };
}

const ADDRESS = {
  projectId: "project_0001",
  issueId: "issue_0001",
  planId: "issue_plan_0001",
} as const;

function ModalHarness({
  onOpenAttempt,
}: {
  onOpenAttempt: (target: PlanOverviewOpenTarget) => void;
}) {
  const [open, setOpen] = useState(true);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        重新打开
      </button>
      {open ? (
        <CodingPlanOverviewModal
          {...ADDRESS}
          onOpenAttempt={onOpenAttempt}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}

function renderModal(onOpenAttempt = vi.fn()) {
  return render(<ModalHarness onOpenAttempt={onOpenAttempt} />);
}

describe("CodingPlanOverviewModal in-page plan overview (F-B)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("renders the plan overview inside a modal dialog on the current page", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify(overviewDto()), { status: 200 }),
      ),
    );

    renderModal();

    const dialog = await screen.findByTestId("plan-overview-modal");
    expect(dialog).toHaveAttribute("role", "dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveAttribute("aria-label", "Coding Plan 总览");
    // 复用 UI-1 的可嵌入总览视图（含聚合确认区）。
    expect(screen.getByTestId("plan-overview-view")).toBeInTheDocument();
    expect(screen.getByTestId("plan-overview-stats")).toBeInTheDocument();
    const list = await screen.findByTestId("plan-overview-units");
    expect(list.querySelectorAll("li")).toHaveLength(2);
    // 浮层行为内核：body 滚动锁 + 焦点移入弹窗卡片。
    expect(document.body.style.overflow).toBe("hidden");
    expect(dialog).toHaveFocus();
  });

  it("closes on Esc and on the close button", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify(overviewDto()), { status: 200 }),
      ),
    );

    renderModal();
    await screen.findByTestId("plan-overview-modal");

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("plan-overview-modal")).toBeNull();
    // 关闭后 body 滚动锁解除、焦点归还触发元素。
    expect(document.body.style.overflow).not.toBe("hidden");

    fireEvent.click(screen.getByRole("button", { name: "重新打开" }));
    await screen.findByTestId("plan-overview-modal");
    fireEvent.click(screen.getByRole("button", { name: "关闭全部 WI 总览" }));
    expect(screen.queryByTestId("plan-overview-modal")).toBeNull();
  });

  it("stops the 5s polling as soon as the modal closes", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn(
      async () => new Response(JSON.stringify(overviewDto()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    renderModal();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);

    // 弹窗可见期间按 5 秒轮询。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(2);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("plan-overview-modal")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("hands WI drill-down back to the host instead of mounting a second attempt instance", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify(overviewDto()), { status: 200 }),
      ),
    );
    const onOpenAttempt = vi.fn();

    renderModal(onOpenAttempt);

    await screen.findByTestId("plan-overview-units");
    fireEvent.click(screen.getByTestId("plan-overview-open-wi_registration"));

    expect(onOpenAttempt).toHaveBeenCalledTimes(1);
    expect(onOpenAttempt).toHaveBeenCalledWith({
      attemptId: "coding_attempt_web_0001",
      workItemId: "wi_registration",
    });
    // 弹窗内不展开第二个 attempt 详情（会双挂载 WS/store）。
    expect(screen.queryAllByTestId("coding-workspace-page")).toHaveLength(0);
  });
});
