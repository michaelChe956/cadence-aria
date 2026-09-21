import { newCommandId } from "../hooks/useWorkspaceWs";
import { gateActionBlockReason, gateTerminateBlockReason } from "./workspace-cockpit-projection";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export type CockpitActionFacade = {
  confirm(): boolean | void;
  /**
   * F-31（v37 复验 #2）：story/design author 门「确认并评审」——复用 confirm
   * 通道携带 with_review=true（HTTP confirm 端点），与主区门卡语义一致。
   */
  confirmReview(): boolean | void;
  feedback(feedback: string): boolean | void;
  terminate(): boolean | void;
  advance(): boolean | void;
  /**
   * v40 复验 #3：author 门「采纳 Review 意见」——把最新 review 报告预填为修订
   * 反馈并切回对话视图（纯客户端动作，无 WS/HTTP 帧），与主区/产物审核面板
   * 按钮同款行为；待处理抽屉由此补齐第四动作。
   */
  adoptReview(): void;
};


export function actionFacadeForFlowKind(
  flowKind: WorkspaceWsState["flowKind"],
): "typed" | "legacy" {
  return flowKind === "single_candidate" ? "typed" : "legacy";
}

export function createCockpitActionFacade(input: {
  flowKind: WorkspaceWsState["flowKind"];
  commandId: string | null;
  getState: () => WorkspaceWsState;
  sendConfirm: (withReview?: boolean) => boolean;
  sendAbandonGate: (commandId: string) => boolean;
  sendHumanGateFeedback: (feedback: string, commandId?: string) => boolean;
  sendAdvance: (commandId?: string) => boolean;
  /** v40 复验 #3：客户端采纳通道（预填修订反馈+切视图），由页面接线。 */
  adoptReview: () => void;
}): CockpitActionFacade {
  return {
    confirm() {
      if (gateActionBlockReason(input.getState()) !== null) {
        return false;
      }
      return input.sendConfirm();
    },
    confirmReview() {
      // F-31：与 confirm 同源阻断判据（收口/相位/终态），仅多带 with_review。
      if (gateActionBlockReason(input.getState()) !== null) {
        return false;
      }
      return input.sendConfirm(true);
    },
    feedback(feedback) {
      if (gateActionBlockReason(input.getState()) !== null) {
        return false;
      }
      if (actionFacadeForFlowKind(input.flowKind) !== "typed") {
        return false;
      }
      // 协议依据（cadence/reports/workitem-conversational-gate-advance/evidence/
      // amendment-wire-notes.md §40-42）：human_gate_feedback 的 command_id 完全由
      // driver/client 生成；服务端按 (session_id, command_id) durable 查重并开新
      // turn（Reserved+扣预算）。重连/刷新后 typed 门只剩 session_state 快照、无活
      // turn 提供既有 command_id 时，凭新生成的 command_id 提交反馈而非拒发。
      // 有活 turn 时仍复用其 command_id（重试/重连重放同 id，不重新生成）。
      return input.sendHumanGateFeedback(feedback, input.commandId ?? newCommandId());
    },
    terminate() {
      // L1 typed 重承载（REQ-RET-02）：终止=显式 abandon_human_gate 命令；
      // 重连/刷新后无活 turn command_id 时凭新 id 提交（与 feedback 同款纪律）。
      // F-21：用终止专属判据——plan 会话停在 human_confirm 的 context blocker/
      // author 失败/缺相位门此前被 phase_mismatch 静默拦截（点击零 WS 出站），
      // 而矩阵与引擎对这些形态均接受 abandon（confirm/feedback 纪律不变）。
      if (gateTerminateBlockReason(input.getState()) !== null) {
        return false;
      }
      return input.sendAbandonGate(input.commandId ?? newCommandId());
    },
    advance() {
      // k3 P2-2：AuthorConfirm 矩阵只放行 Abort/AbandonHumanGate——HTTP confirm 的
      // 乐观 confirmed 态也不发 advance（否则必回 ADVANCE_STAGE_INVALID 红条）。
      if (input.getState().stage === "author_confirm") {
        return false;
      }
      const reason = gateActionBlockReason(input.getState());
      if (reason !== null && reason !== "closed") {
        return false;
      }
      return input.sendAdvance(newCommandId());
    },
    adoptReview() {
      // v40 复验 #3：与 confirm 同源阻断判据——门已收口/相位漂移时不再把修订
      // 反馈预填进不可回传的输入框；纯客户端动作，无 WS/HTTP 帧。
      if (gateActionBlockReason(input.getState()) !== null) {
        return;
      }
      input.adoptReview();
    },
  };
}


export type ProtocolErrorDisposition =
  | { kind: "gate"; turnId: string }
  | { kind: "advance"; commandId: string }
  | { kind: "hard_error" };

type ProtocolRoutingState = Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands">;

const GATE_REJECTION_CODES: Record<string, true> = {
  INVALID_HUMAN_CONFIRM_ACTION: true,
  HUMAN_GATE_NOT_READY: true,
  HUMAN_GATE_FEEDBACK_REJECTED: true,
};

const ADVANCE_REPLAY_CODES: Record<string, true> = {
  ADVANCE_REPLAY_NOT_READY: true,
  ADVANCE_REPLAY_INCOMPLETE: true,
};

export function classifyProtocolError(
  code: string,
  context: unknown,
  state: ProtocolRoutingState,
): ProtocolErrorDisposition {
  const turnId = contextString(context, "turn_id");
  if (
    turnId !== null &&
    GATE_REJECTION_CODES[code] === true &&
    state.humanGateTurn?.turn_id === turnId
  ) {
    return { kind: "gate", turnId };
  }

  const commandId = contextString(context, "command_id");
  if (
    commandId !== null &&
    ADVANCE_REPLAY_CODES[code] === true &&
    state.advanceCommands[commandId] !== undefined
  ) {
    return { kind: "advance", commandId };
  }

  return { kind: "hard_error" };
}

export function gateIdentityFromState(
  state: Pick<WorkspaceWsState, "humanGateTurn" | "humanGateSnapshot" | "stage">,
): string | null {
  if (state.humanGateTurn) {
    return state.humanGateTurn.turn_id;
  }
  if (state.humanGateSnapshot?.opened_at) {
    return `snapshot:${state.humanGateSnapshot.opened_at}:${snapshotGateFingerprint(state.humanGateSnapshot)}`;
  }
  if (state.humanGateSnapshot) {
    return `snapshot:${state.stage}`;
  }
  // L1 重承载（REQ-RET-02）：gateId 派生收敛为 humanGateTurn/humanGateSnapshot 两路；
  // stage-only legacy 门不再派生 `legacy:` 前缀 id（投影 key 由 selectGateProjection
  // 以 `stage:` 前缀兜底）。
  return null;
}

export function snapshotGateFingerprint(
  snapshot: Pick<
    NonNullable<WorkspaceWsState["humanGateSnapshot"]>,
    "trigger" | "manual_repairs_remaining" | "resumable" | "repeated_fingerprints" | "findings"
  >,
): string {
  return `${snapshot.trigger}|${snapshot.manual_repairs_remaining}|${snapshot.resumable}|${snapshot.repeated_fingerprints.join(",")}|${snapshot.findings.map((finding) => finding.fingerprint).join(",")}`;
}

function contextString(context: unknown, key: string): string | null {
  if (typeof context !== "object" || context === null) {
    return null;
  }
  const value = (context as Record<string, unknown>)[key];
  return typeof value === "string" ? value : null;
}
