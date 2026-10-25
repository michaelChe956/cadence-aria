// change `triage-gate-actions-by-verdict` 前端契约回归。
//
// 三个 Code Review 分诊门的动作集合由服务端按该 attempt 最新一条 Code Review 结论
// 的 verdict 派生：基础恒为 retry_review/manual_continue/abort，仅 verdict ∈
// {request_changes, blocked} 时含 send_to_coder。前端 MUST 严格按服务端
// `available_actions` 渲染——不增删动作、不按错误码置灰（F-D 的过渡置灰已移除）；
// 「为什么没有送回 Coder」的说明文案取自门记录暴露的 `latest_review_verdict`
//（无该事实时给通用说明），不依赖任何提交失败往返。
//
// 保留 F-E：必填校验前置（空说明即置灰 + 就地说明动作名与后果）、「人工放行说明
// （记入质量豁免审计）」标签与放行/修复意见分开命名。

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useCodingWorkspaceStore,
  type CodingPendingGate,
} from "../state/coding-workspace-store";
import { GatePanel } from "./CodingWorkspaceControls";

/** 四项门：最新 verdict=request_changes（可返修）→ 服务端给四项。 */
const requestChangesTriageGate: CodingPendingGate = {
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
  latest_review_verdict: "request_changes",
  available_actions: [
    { action_id: "retry_review", label: "重试代码审查", action_type: "retry_review" },
    { action_id: "send_to_coder", label: "提交给 Coder 修复", action_type: "send_to_coder" },
    { action_id: "manual_continue", label: "人工继续", action_type: "manual_continue" },
    { action_id: "abort", label: "终止", action_type: "abort" },
  ],
};

/** 三项门：最新 verdict=approve → 服务端派生 retry_review/manual_continue/abort。 */
const approveTriageGate: CodingPendingGate = {
  ...requestChangesTriageGate,
  gate_id: "coding_blocked_gate_triage_0002",
  latest_review_verdict: "approve",
  available_actions: [
    { action_id: "retry_review", label: "重试代码审查", action_type: "retry_review" },
    { action_id: "manual_continue", label: "人工继续", action_type: "manual_continue" },
    { action_id: "abort", label: "终止", action_type: "abort" },
  ],
};

/** 三项门 + 派生事实缺失（结论读不到 / 升级前落地的存量门没有该字段）。 */
const unknownVerdictTriageGate: CodingPendingGate = {
  ...approveTriageGate,
  gate_id: "coding_blocked_gate_triage_0003",
  latest_review_verdict: null,
};

/** 只含 retry_review + manual_continue 的门（F-E 必填文案用例）。 */
const manualOnlyTriageGate: CodingPendingGate = {
  ...approveTriageGate,
  gate_id: "coding_blocked_gate_triage_0004",
  available_actions: [
    { action_id: "retry_review", label: "重试代码审查", action_type: "retry_review" },
    { action_id: "manual_continue", label: "人工继续", action_type: "manual_continue" },
  ],
};

/** 不属于三个分诊门的四动作门（`reviewer_rework_limit_reached`）——动作集合不受
 * verdict 派生，也不该出现「为什么没有送回 Coder」的派生说明。 */
const coderFeedbackGate: CodingPendingGate = {
  ...requestChangesTriageGate,
  gate_id: "coding_blocked_gate_rework_0001",
  title: "Code Review 修复超上限",
  reason_code: "reviewer_rework_limit_reached",
  latest_review_verdict: null,
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

const OMITTED_TEST_ID = "coding-gate-send-to-coder-omitted";

describe("GatePanel：动作严格按服务端 available_actions 渲染", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("四项门：后端给四项就渲染四项，不删动作、不预置任何派生说明", async () => {
    const user = userEvent.setup();
    renderGatePanel(requestChangesTriageGate);

    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent(
      "Code Review 结论需人工分诊",
    );
    // 空说明时 send_to_coder/manual_continue 按 F-E 前置必填置灰。
    expect(screen.getByRole("button", { name: "提交给 Coder 修复" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "重试代码审查" })).toBeEnabled();

    await user.type(screen.getByLabelText("人工处理说明（人工继续会记入质量豁免审计）"), "结论可返修");

    for (const action of requestChangesTriageGate.available_actions) {
      expect(screen.getByRole("button", { name: action.label })).toBeEnabled();
    }
    // 动作在集合内 → 没有「缺少该动作」的说明。
    expect(screen.queryByTestId(OMITTED_TEST_ID)).toBeNull();
  });

  it("三项门（verdict=approve）：不渲染「提交给 Coder 修复」，其余三项不受影响", () => {
    renderGatePanel(approveTriageGate);

    expect(screen.queryByRole("button", { name: "提交给 Coder 修复" })).toBeNull();
    for (const label of ["重试代码审查", "人工继续", "终止"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("三项门首次渲染即按门记录的最新 verdict 给说明（无失败往返）", () => {
    renderGatePanel(approveTriageGate);

    const guidance = screen.getByTestId(OMITTED_TEST_ID);
    expect(guidance).toHaveTextContent("本门动作集合不含「提交给 Coder 修复」");
    expect(guidance).toHaveTextContent("verdict=approve");
    // 替代动作引用本门真实动作文案。
    expect(guidance).toHaveTextContent("「重试代码审查」");
    expect(guidance).toHaveTextContent("「人工继续」");
    expect(guidance).toHaveTextContent("质量豁免");
    // 不依赖任何错误码（渲染前没有任何被拒绝的提交）。
    expect(screen.getByTestId("coding-pending-gate")).not.toHaveTextContent(
      "send_to_coder_latest_review_not_actionable",
    );
  });

  it("派生事实缺失（存量在途门／结论读不到）：给通用说明，仍只渲染三项", () => {
    renderGatePanel(unknownVerdictTriageGate);

    const guidance = screen.getByTestId(OMITTED_TEST_ID);
    expect(guidance).toHaveTextContent("最近一次 Code Review 结论不可读");
    expect(guidance).not.toHaveTextContent("verdict=approve");
    expect(screen.queryByRole("button", { name: "提交给 Coder 修复" })).toBeNull();
  });

  it("非分诊门不出现派生说明（动作集合不按 verdict 派生）", () => {
    renderGatePanel(coderFeedbackGate);
    expect(screen.queryByTestId(OMITTED_TEST_ID)).toBeNull();
    expect(screen.getByRole("button", { name: "提交给 Coder 修复" })).toBeInTheDocument();
  });

  it("服务端已拒绝过的错误码不再置灰动作、不给专用指引（过渡逻辑已移除）", async () => {
    const user = userEvent.setup();
    // 陈旧快照：verdict=approve 但服务端仍给了四动作 → 客户端照服务端渲染，
    // 不自行按 verdict 或错误码禁用/删除动作。
    const staleGate: CodingPendingGate = {
      ...requestChangesTriageGate,
      latest_review_verdict: "approve",
      submitting: false,
      errorCode: "coding_gate_response_failed",
    };
    useCodingWorkspaceStore.getState().setProtocolError({
      code: "coding_gate_response_failed",
      message: SEND_TO_CODER_NOT_ACTIONABLE_MESSAGE,
    });
    renderGatePanel(staleGate);

    await user.type(screen.getByLabelText("人工处理说明（人工继续会记入质量豁免审计）"), "人工核对");

    const sendToCoder = screen.getByRole("button", { name: "提交给 Coder 修复" });
    expect(sendToCoder).toBeEnabled();
    expect(sendToCoder).not.toHaveAttribute("title");
    // 动作在集合内 → 不出现「缺少该动作」的说明；原始粗码仍作为技术信息展示。
    expect(screen.queryByTestId(OMITTED_TEST_ID)).toBeNull();
    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent(
      "coding_gate_response_failed",
    );
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
      "coding_blocked_gate_triage_0004",
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
