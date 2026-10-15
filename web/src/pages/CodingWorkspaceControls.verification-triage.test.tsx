import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  VerificationCommandEvidence,
  VerificationTriageRecord,
} from "../api/types";
import type { CodingPendingGate } from "../state/coding-workspace-store";
import { GatePanel, isTriageCardGate } from "./CodingWorkspaceControls";
const fourActionGate: CodingPendingGate = {
  gate_id: "coding_blocked_gate_0001",
  kind: "blocked",
  title: "验证证据不完整",
  description: "code review 验证不完整，等待人工处理",
  stage: "code_review",
  role: "code_reviewer",
  expires_at: null,
  provider_snapshot: null,
  available_actions: [
    { action_id: "retry_review", label: "重试评审", action_type: "retry_review" },
    { action_id: "send_to_coder", label: "提交给 Coder 修复", action_type: "send_to_coder" },
    { action_id: "manual_continue", label: "人工继续", action_type: "manual_continue" },
    { action_id: "abort", label: "中止", action_type: "abort" },
  ],
  reason_code: "code_review_verification_incomplete",
  evidence_refs: [],
  raw_provider_output_ref: null,
  diagnostic: null,
};

function renderGatePanel(
  gate: CodingPendingGate,
  props?: {
    verificationTriage?: VerificationTriageRecord | null;
    onEnterVerificationTriage?: (gateId: string) => void;
    commandEvidence?: VerificationCommandEvidence | null;
    onRerunPlannedCommand?: (checkId: string) => void;
  },
) {
  return render(
    <GatePanel
      gate={gate}
      onRespond={vi.fn()}
      onConfirmStage={vi.fn()}
      onAbort={vi.fn()}
      verificationTriage={props?.verificationTriage ?? null}
      onEnterVerificationTriage={props?.onEnterVerificationTriage}
      commandEvidence={props?.commandEvidence ?? null}
      onRerunPlannedCommand={props?.onRerunPlannedCommand}
    />,
  );
}

describe("CodingWorkspaceControls verification triage panel（C2 Task 8/#19）", () => {
  it("eligible gate 渲染旁路面板且不进 available_actions，点击转入携带 gate_id", () => {
    const onEnter = vi.fn();
    renderGatePanel(fourActionGate, { onEnterVerificationTriage: onEnter });

    // 原门动作集合保持恰四动作（旁路入口不追加为 action 按钮）。
    expect(
      fourActionGate.available_actions
        .map((action) => action.action_id)
        .sort()
        .join(","),
    ).toBe("abort,manual_continue,retry_review,send_to_coder");
    for (const action of fourActionGate.available_actions) {
      expect(screen.getByRole("button", { name: action.label })).toBeTruthy();
    }

    const panel = screen.getByTestId("coding-verification-triage");
    expect(panel.textContent).toContain("验证处理");
    const enterButton = screen.getByTestId("coding-verification-triage-enter");
    expect(screen.queryByTestId("coding-verification-triage-status")).toBeNull();

    fireEvent.click(enterButton);
    expect(onEnter).toHaveBeenCalledWith("coding_blocked_gate_0001");
  });

  it("已有批准记录时显示状态与 finding 覆盖标注，不再提供转入入口", () => {
    renderGatePanel(fourActionGate, {
      verificationTriage: {
        triage_id: "verification_triage_0001",
        attempt_id: "coding_attempt_0001",
        finding_id: "code_review_report_0001#0",
        check_id: "check_nonzero",
        plan_revision_id: "work_item_revision_0001",
        original_command: null,
        alternative_command: "pnpm -C web exec vitest run src/lib.test.ts",
        cwd: "/repo",
        outcome: "3 passed",
        test_execution_count: 3,
        environment: "linux",
        scope: ["check_nonzero"],
        expires_at: "2026-10-06T00:00:00Z",
        status: "approved",
        conclusion: "accept_equivalent_evidence",
        reason: "等价证据成立",
        decided_by: "operator-1",
        decided_at: "2026-09-29T00:00:00Z",
      },
    });

    const status = screen.getByTestId("coding-verification-triage-status");
    expect(status.textContent).toContain("已批准");
    expect(status.textContent).toContain("finding 已由验证处理覆盖");
    expect(screen.queryByTestId("coding-verification-triage-enter")).toBeNull();
  });

  it("非可转入门不渲染验证处理面板", () => {
    renderGatePanel({
      ...fourActionGate,
      reason_code: "reviewer_rework_limit_reached",
    });
    expect(screen.queryByTestId("coding-verification-triage")).toBeNull();
  });
});

describe("CodingWorkspaceControls command evidence panel（C2 Task 9/#2）", () => {
  it("并列显示计划与实际命令，mismatch 标注不一致并给出重跑入口携带 check_id", () => {
    const onRerun = vi.fn();
    renderGatePanel(fourActionGate, {
      commandEvidence: {
        check_id: "check_plain",
        planned_command: "cargo test --lib",
        planned_manual_instruction: null,
        actual_command: "cargo test --lib --features strict",
        actual_cwd: "/repo/worktree",
        exit_code: 1,
        test_execution_count: null,
        environment_summary: null,
        mismatch: true,
      },
      onRerunPlannedCommand: onRerun,
    });

    const panel = screen.getByTestId("coding-command-evidence");
    expect(panel.textContent).toContain("cargo test --lib");
    expect(panel.textContent).toContain("cargo test --lib --features strict");
    expect(panel.textContent).toContain("/repo/worktree");
    expect(
      screen.getByTestId("coding-command-evidence-mismatch").textContent,
    ).toContain("实际执行命令与计划不一致");

    fireEvent.click(screen.getByTestId("coding-rerun-planned-command"));
    expect(onRerun).toHaveBeenCalledWith("check_plain");
  });

  it("无实际命令记录显示未记录且不提供重跑入口", () => {
    renderGatePanel(fourActionGate, {
      commandEvidence: {
        check_id: "check_plain",
        planned_command: "cargo test --lib",
        planned_manual_instruction: null,
        actual_command: null,
        actual_cwd: null,
        exit_code: null,
        test_execution_count: null,
        environment_summary: null,
        mismatch: false,
      },
    });
    const panel = screen.getByTestId("coding-command-evidence");
    expect(panel.textContent).toContain("实际命令：未记录");
    expect(screen.queryByTestId("coding-command-evidence-mismatch")).toBeNull();
    expect(screen.queryByTestId("coding-rerun-planned-command")).toBeNull();
  });
});

describe("CodingWorkspaceControls GatePanel 分诊卡分流（P2-d/REQ-CRO-09）", () => {
  const operationalBlockerGate: CodingPendingGate = {
    ...fourActionGate,
    title: "运行环境阻塞",
    description: "Coder 报告运行环境阻塞，流程已暂停",
    stage: "coding",
    role: "coder",
    reason_code: "coding_operational_blocker",
    available_actions: [
      { action_id: "retry_coding", label: "重新启动 Coder", action_type: "retry_coding" },
      {
        action_id: "retry_coding_with_context",
        label: "我来说明情况，让 AI 继续",
        action_type: "retry_coding_with_context",
      },
      { action_id: "abort", label: "终止", action_type: "abort" },
    ],
  };

  it("isTriageCardGate 按 reason_code 判定（C5：coder_provider_stalled 的纳入由 P4-e 扩）", () => {
    expect(isTriageCardGate(operationalBlockerGate)).toBe(true);
    expect(
      isTriageCardGate({ ...operationalBlockerGate, reason_code: "coding_output_human_triage" }),
    ).toBe(true);
    expect(isTriageCardGate(fourActionGate)).toBe(false);
    expect(
      isTriageCardGate({ ...operationalBlockerGate, reason_code: null }),
    ).toBe(false);
  });

  it("分诊门经 GatePanel 分流到 CoderTriageCard；CR 门保持原渲染（回归）", () => {
    const { unmount: unmountCard } = renderGatePanel(operationalBlockerGate);
    expect(screen.getByTestId("coder-triage-card")).toBeInTheDocument();
    expect(screen.queryByTestId("coding-pending-gate")).toBeNull();
    unmountCard();

    renderGatePanel(fourActionGate);
    expect(screen.getByTestId("coding-pending-gate")).toBeInTheDocument();
    expect(screen.queryByTestId("coder-triage-card")).toBeNull();
  });

  it("coding_operational_blocker 已进验证处理资格白名单：折叠展开后可转入（k3 放行条件 ①）", () => {
    const onEnter = vi.fn();
    renderGatePanel(operationalBlockerGate, { onEnterVerificationTriage: onEnter });

    const technical = screen.getByTestId(
      "coder-triage-technical-details",
    ) as HTMLDetailsElement;
    expect(technical.open).toBe(false);
    fireEvent.click(within(technical).getByText("技术详情"));
    const advanced = screen.getByTestId("coder-triage-advanced-verification");
    fireEvent.click(within(advanced).getByText("高级：验证处理"));

    const panel = screen.getByTestId("coding-verification-triage");
    expect(panel.textContent).toContain("验证处理");
    fireEvent.click(screen.getByTestId("coding-verification-triage-enter"));
    expect(onEnter).toHaveBeenCalledWith("coding_blocked_gate_0001");
  });
});
