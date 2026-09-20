import { newCommandId } from "../hooks/useWorkspaceWs";
import { gateActionBlockReason } from "./workspace-cockpit-projection";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export type CockpitActionFacade = {
  confirm(): boolean | void;
  feedback(feedback: string): boolean | void;
  terminate(): boolean | void;
  advance(): boolean | void;
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
  sendConfirm: () => boolean;
  sendAbandonGate: (commandId: string) => boolean;
  sendHumanGateFeedback: (feedback: string, commandId?: string) => boolean;
  sendAdvance: (commandId?: string) => boolean;
}): CockpitActionFacade {
  return {
    confirm() {
      if (gateActionBlockReason(input.getState()) !== null) {
        return false;
      }
      return input.sendConfirm();
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
      if (gateActionBlockReason(input.getState()) !== null) {
        return false;
      }
      // L1 typed 重承载（REQ-RET-02）：终止=显式 abandon_human_gate 命令；
      // 重连/刷新后无活 turn command_id 时凭新 id 提交（与 feedback 同款纪律）。
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
