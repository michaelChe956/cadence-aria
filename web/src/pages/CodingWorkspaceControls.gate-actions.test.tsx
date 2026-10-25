// F-D / F-E 回归（owner 2026-10-25 现场截图 + 真机复现）。
//
// F-D：Code Review 分诊门（reason_code=code_review_output_human_triage）上的
//   「提交给 Coder 修复」必然失败（服务端 `send_to_coder_latest_review_not_actionable`：
//   该动作的前置是 verdict ∈ {request_changes, blocked}，而本门的结论恰恰不可机械判定）。
//   spec（coding-code-review-triage / coding-verification-triage）要求三个分诊门
//   **恰好提供四动作** → 动作集合不动（不从 available_actions 删除），改为：
//   在**发生该失败的那个门实例**上把该动作置灰 + 就地给可执行指引；重试/门解决/
//   新门实例（errorCode 被清空）即自动复位。
// F-E：空说明点「人工继续」→ 提示指代不清（把放行说明说成"修复意见"、点一次才知道要填）。
//   现在：必填校验前置（就地提示 + 动作置灰）、文案带动作名与后果、放行说明与
//   发回 Coder 的修复意见分开命名。

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useCodingWorkspaceStore,
  type CodingPendingGate,
} from "../state/coding-workspace-store";
import { GatePanel } from "./CodingWorkspaceControls";

/** 服务端权威动作集合（spec 四动作）：本文件的门夹具保持四动作，验证前端不删动作。 */
const codeReviewTriageGate: CodingPendingGate = {
  gate_id: "coding_blocked_gate_triage_0001",
  kind: "blocked",
  title: "Code Review 结论需人工分诊",
  description: "review 输出无法机械判定为自动判决",
  stage: "code_review",
  role: "code_reviewer",
  expires_at: null,
  provider_snapshot: null,
  reason_code: "code_review_output_human_triage",
  evidence_refs: ["code_review_report_0003"],
  raw_provider_output_ref: null,
  diagnostic: null,
  available_actions: [
    { action_id: "retry_review", label: "重试代码审查", action_type: "retry_review" },
    { action_id: "send_to_coder", label: "提交给 Coder 修复", action_type: "send_to_coder" },
    { action_id: "manual_continue", label: "人工继续", action_type: "manual_continue" },
    { action_id: "abort", label: "终止", action_type: "abort" },
  ],
};

const manualOnlyTriageGate: CodingPendingGate = {
  ...codeReviewTriageGate,
  gate_id: "coding_blocked_gate_triage_0002",
  available_actions: [
    { action_id: "retry_review", label: "重试代码审查", action_type: "retry_review" },
    { action_id: "manual_continue", label: "人工继续", action_type: "manual_continue" },
  ],
};

const coderFeedbackGate: CodingPendingGate = {
  ...codeReviewTriageGate,
  gate_id: "coding_blocked_gate_rework_0001",
  title: "Code Review 修复超上限",
  reason_code: "reviewer_rework_limit_reached",
  available_actions: [
    { action_id: "send_to_coder", label: "提交给 Coder 修复", action_type: "send_to_coder" },
    { action_id: "abort", label: "终止", action_type: "abort" },
  ],
};

const SEND_TO_CODER_NOT_ACTIONABLE_MESSAGE =
  "coding_provider_stream_failed: send_to_coder_latest_review_not_actionable";

function renderGatePanel(gate: CodingPendingGate, onRespond = vi.fn()) {
  const body = (nextGate: CodingPendingGate) => (
    <GatePanel
      gate={nextGate}
      onRespond={onRespond}
      onConfirmStage={vi.fn()}
      onAbort={vi.fn()}
    />
  );
  return { onRespond, ...render(body(gate)), body };
}

describe("GatePanel F-D：必败动作按服务端事实置灰且可复位", () => {
  // 四动作门上两种上下文动作并存 → F-E 的字段名取"两者并存"文案。
  const reasonLabel = "人工处理说明（人工继续会记入质量豁免审计）";

  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("初次渲染：四动作全部保留；填好说明后全部可用，且无 F-D 指引（不预先猜）", async () => {
    const user = userEvent.setup();
    renderGatePanel(codeReviewTriageGate);

    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent(
      "Code Review 结论需人工分诊",
    );
    // 空说明时按 F-E 前置必填置灰（与 F-D 的服务器判定无关：无指引块、无 title 解释）。
    expect(screen.getByRole("button", { name: "提交给 Coder 修复" })).toBeDisabled();
    expect(screen.queryByTestId("coding-gate-send-to-coder-unavailable")).toBeNull();

    await user.type(screen.getByLabelText(reasonLabel), "结论不可判定，人工核对");

    for (const action of codeReviewTriageGate.available_actions) {
      expect(screen.getByRole("button", { name: action.label })).toBeEnabled();
    }
    expect(screen.queryByTestId("coding-gate-send-to-coder-unavailable")).toBeNull();
  });

  it("服务端拒绝 send_to_coder 后：该实例置灰 + 可执行指引 + 原始码保留，重试后复位", async () => {
    const user = userEvent.setup();
    const { rerender, body } = renderGatePanel(codeReviewTriageGate);

    await user.type(screen.getByLabelText(reasonLabel), "结论不可判定，人工核对");

    // 用户点了「提交给 Coder 修复」，服务端以该原因拒绝（WS 帧 code 是粗码，
    // message 带具体原因 → store 落成 gate.errorCode + protocolError）。
    useCodingWorkspaceStore.getState().setProtocolError({
      code: "coding_gate_response_failed",
      message: SEND_TO_CODER_NOT_ACTIONABLE_MESSAGE,
    });
    rerender(
      body({
        ...codeReviewTriageGate,
        submitting: false,
        errorCode: "coding_gate_response_failed",
      }),
    );

    const rejected = screen.getByRole("button", { name: "提交给 Coder 修复" });
    expect(rejected).toBeDisabled();
    expect(rejected).toHaveAttribute(
      "title",
      "最近一次 Code Review 结论无法作为返修指令；请重试代码审查或人工继续",
    );
    // 动作集合仍是四项（spec MUST 恰好四动作）：只是这一实例的该动作被置灰。
    for (const label of ["重试代码审查", "人工继续", "终止"]) {
      expect(screen.getByRole("button", { name: label })).toBeEnabled();
    }

    // 可执行指引：说清"为什么不可用 + 该用什么替代"。
    const guidance = screen.getByTestId("coding-gate-send-to-coder-unavailable");
    expect(guidance).toHaveTextContent("「提交给 Coder 修复」在当前门不可用");
    expect(guidance).toHaveTextContent("「重试代码审查」");
    expect(guidance).toHaveTextContent("「人工继续」");
    // 原始错误码保留为技术信息，不再作为唯一解释。
    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent(
      "coding_gate_response_failed",
    );

    // 复位：重试代码审查/人工继续提交时 store 清 errorCode；新门实例也只带自己的
    // errorCode → 该动作随实例走，不会被一次失败永久废掉。
    await user.click(screen.getByRole("button", { name: "重试代码审查" }));
    rerender(body({ ...codeReviewTriageGate, submitting: true, errorCode: null }));
    rerender(body({ ...codeReviewTriageGate, submitting: false, errorCode: null }));

    expect(screen.getByRole("button", { name: "提交给 Coder 修复" })).toBeEnabled();
    expect(screen.queryByTestId("coding-gate-send-to-coder-unavailable")).toBeNull();
  });

  it("同一粗码但原因不是「结论不可 action」时不置灰（不过度封锁）", async () => {
    const user = userEvent.setup();
    useCodingWorkspaceStore.getState().setProtocolError({
      code: "coding_gate_response_failed",
      message: "coding_failed_review_recovery_requires_reservation",
    });
    renderGatePanel({
      ...codeReviewTriageGate,
      submitting: false,
      errorCode: "coding_gate_response_failed",
    });

    await user.type(screen.getByLabelText(reasonLabel), "结论不可判定，人工核对");

    expect(screen.getByRole("button", { name: "提交给 Coder 修复" })).toBeEnabled();
    expect(screen.queryByTestId("coding-gate-send-to-coder-unavailable")).toBeNull();
  });
});

describe("GatePanel F-E：「人工继续」的放行说明必填前置且指代清楚", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("人工放行门：字段名为放行说明、空值即置灰并就地说明动作名与后果", async () => {
    const user = userEvent.setup();
    const { onRespond } = renderGatePanel(manualOnlyTriageGate);

    const field = screen.getByLabelText("人工放行说明（记入质量豁免审计）");
    expect(field).toBeInTheDocument();

    const hint = screen.getByTestId("coding-gate-context-required");
    expect(hint).toHaveTextContent("「人工继续」需要填写人工放行说明");
    expect(hint).toHaveTextContent("记入质量豁免审计");
    expect(hint).toHaveTextContent("reason_code=code_review_output_human_triage");
    // 不与「提交给 Coder 修复」路径的修复意见混淆。
    expect(hint).not.toHaveTextContent("人工修复意见");

    // 必填校验前置：空说明时该动作直接置灰，无需先点一次。
    expect(screen.getByRole("button", { name: "人工继续" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "重试代码审查" })).toBeEnabled();

    await user.type(field, "review 结论与改动不一致，人工核对后放行，后续补 E2E");
    expect(screen.getByRole("button", { name: "人工继续" })).toBeEnabled();
    expect(screen.queryByTestId("coding-gate-context-required")).toBeNull();

    await user.click(screen.getByRole("button", { name: "人工继续" }));
    expect(onRespond).toHaveBeenCalledWith(
      "coding_blocked_gate_triage_0002",
      "manual_continue",
      "review 结论与改动不一致，人工核对后放行，后续补 E2E",
    );
  });

  it("发回 Coder 门仍叫「人工修复意见」，文案带动作名与后果且不提质量豁免", () => {
    renderGatePanel(coderFeedbackGate);

    expect(screen.getByLabelText("人工修复意见")).toBeInTheDocument();
    const hint = screen.getByTestId("coding-gate-context-required");
    expect(hint).toHaveTextContent("「提交给 Coder 修复」需要填写人工修复意见");
    expect(hint).toHaveTextContent("将作为本轮修复意见发给 Coder");
    expect(hint).not.toHaveTextContent("质量豁免");
    expect(screen.getByTestId("coding-pending-gate")).not.toHaveTextContent("质量豁免");

    expect(screen.getByRole("button", { name: "提交给 Coder 修复" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "终止" })).toBeEnabled();
  });
});
