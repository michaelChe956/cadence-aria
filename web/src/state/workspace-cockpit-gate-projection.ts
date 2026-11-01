import { gateIdentityFromState } from "./cockpit-action-routing";
import type {
  WorkItemPlanCandidateCompileFailure,
  WorkItemPlanHumanGateSnapshot,
} from "../api/types";
import type { ChatEntry } from "./chat-entries";
import type {
  GateClosureDecision,
  HumanGateTurnState,
  TimelineNode,
  WorkspaceWsState,
} from "./workspace-ws-store-types";

// 从 workspace-cockpit-projection.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 门投影（门种类判定 / 阻断判据 / 门卡公共投影）及其类型。
export const GATE_TRIGGER_LABELS: Record<WorkItemPlanHumanGateSnapshot["trigger"], string> = {
  native_human_required: "引擎判定需人工",
  repeated_fingerprint: "同一问题重复出现",
  verification_new_findings: "复验发现新问题",
  repair_budget_exhausted: "修复轮次已用尽",
};

const ADVANCE_REPLAY_CODES = new Set([
  "ADVANCE_REPLAY_NOT_READY",
  "ADVANCE_REPLAY_INCOMPLETE",
]);

export function isAdvanceReplayCode(code: string): boolean {
  return ADVANCE_REPLAY_CODES.has(code);
}

export type GateActionBlockReason =
  | "terminal_stage"
  | "phase_mismatch"
  | "closed"
  | null;

const TERMINAL_GATE_STAGES: Record<string, true> = {
  prepare_context: true,
  running: true,
  author_confirm: true,
  cross_review: true,
  review_decision: true,
  revision: true,
};

/**
 * REQ-PCG-01/02（plan-compile-gate-visibility）：门种类。既有四路（typed turn /
 * durable snapshot / story-design AuthorConfirm / human_confirm 阶段）都是 human
 * gate；批次确认与 compile recovery 是两条独立的 durable 门——没有 typed turn 或
 * snapshot 载体，由引擎落下的 timeline node（+ 阶段/流凭据）识别。
 */
export type GateKind = "human_gate" | "batch_confirm" | "compile_recovery";

/** REQ-PCG-03/F-30：终态会话状态——终态后不得新开或复活任何门。 */
const TERMINAL_SESSION_STATUSES: Record<string, true> = {
  confirmed: true,
  terminated: true,
};

/**
 * F-42：终态会话的门收口裁决。确认链（WS confirm / 对话门 approve / HTTP confirm
 * 200 乐观态）落 Confirmed/Terminated 后不再补发 human_gate_closed 帧，重连后内存
 * closure 也为空——只看 closure，终态门卡会残留可点的确认/终止按钮，二次点击被
 * 服务端矩阵拒（INVALID_MESSAGE_FOR_STAGE: confirm not allowed in stage completed；
 * 实测 workspace_session_0003）。判据与 REQ-PCG-03/F-30 的 TERMINAL_SESSION_STATUSES
 * 同源：终态即关门，裁决由已持久化的会话状态派生（terminated → terminate，其余
 * 终态 → confirm）。
 */
export function terminalGateClosure(
  state: Pick<WorkspaceWsState, "sessionStatus">,
): GateClosureDecision | null {
  const status = state.sessionStatus ?? "";
  if (TERMINAL_SESSION_STATUSES[status] !== true) {
    return null;
  }
  return status === "terminated" ? "terminate" : "confirm";
}

/** 门身份判据的输入面（投影、阻断判据、动作面共用）。 */
export type GateKindState = Pick<
  WorkspaceWsState,
  "stage" | "flowKind" | "timelineNodes"
>;

/**
 * REQ-PCG-01：整组 Draft 确认门（`work_item_batch_confirm`）——SC 流停在
 * AuthorConfirm 阶段的 durable 批次门。条件取自引擎不变式：flow=single_candidate
 * + stage=author_confirm（compile 成功产出整组 Draft 与 compile 失败转批次确认
 * 两条路径都经 `enter_work_item_batch_confirm`，decisions.rs）后仍有 Active 门节点。
 * 凭据不齐（flow/stage 不符）时不认门 → REQ-PCG-03 fail-closed。
 */
function batchConfirmGateNode(state: GateKindState): TimelineNode | null {
  if (state.flowKind !== "single_candidate" || state.stage !== "author_confirm") {
    return null;
  }
  return (
    state.timelineNodes.find(
      (node) =>
        node.node_type === "work_item_batch_confirm" && node.status === "active",
    ) ?? null
  );
}

/**
 * REQ-PCG-02：Final Compile recovery 门（`work_item_plan_compile_recovery`）——
 * 引擎进入 recovery 时先切 stage 再落节点（compile.rs
 * `enter_work_item_plan_compile_recovery`：transition_stage(HumanConfirm) →
 * create_timeline_node），故 HumanConfirm 阶段是门的 durable 凭据之一；阶段不符
 * 即为状态事件不一致 → REQ-PCG-03 fail-closed（不投影、不猜动作），也避免残留
 * 节点在后续 human_confirm 门上冒充 recovery。
 */
export function compileRecoveryGateNode(
  state: Pick<WorkspaceWsState, "stage" | "timelineNodes">,
): TimelineNode | null {
  if (state.stage !== "human_confirm") {
    return null;
  }
  return (
    state.timelineNodes.find(
      (node) =>
        node.node_type === "work_item_plan_compile_recovery" &&
        node.status === "active",
    ) ?? null
  );
}

/** 门种类判定：投影 / 阻断判据 / 动作面共用的单一事实源。 */
export function gateKindOf(state: GateKindState): GateKind {
  if (batchConfirmGateNode(state) !== null) {
    return "batch_confirm";
  }
  if (compileRecoveryGateNode(state) !== null) {
    return "compile_recovery";
  }
  return "human_gate";
}

/** F-20：story/design legacy 流的 AuthorConfirm 阶段本身即人工门（无 typed turn/snapshot）。 */
export function isStoryDesignAuthorConfirm(
  state: Pick<WorkspaceWsState, "stage" | "workspaceType">,
): boolean {
  return (
    state.stage === "author_confirm" &&
    (state.workspaceType === "story" || state.workspaceType === "design")
  );
}

export function gateActionBlockReason(state: WorkspaceWsState): GateActionBlockReason {
  // REQ-PCG-01/02：批次确认与 compile recovery 门没有 typed turn/durable snapshot
  // 载体，阻断判据只取「门已关闭」与「会话终态」（F-30）——不得套用 HumanConfirm
  // 的相位纪律：它们的 stage（AuthorConfirm / HumanConfirm）不是 typed 门阶段，
  // 走通用分支会误报 terminal_stage/phase_mismatch 把整门静默禁用。
  if (gateKindOf(state) !== "human_gate") {
    if (state.humanGateClosure?.decision) {
      return "closed";
    }
    return TERMINAL_SESSION_STATUSES[state.sessionStatus ?? ""] === true
      ? "terminal_stage"
      : null;
  }
  // F-42：closure 缺失但会话已终态（重连/刷新、HTTP confirm 乐观态）同样是关门——
  // 终态判据与 F-30 同源，给出「已关闭」而非放行可点按钮。
  if (state.humanGateClosure?.decision || terminalGateClosure(state) !== null) {
    return "closed";
  }
  if (state.stage === "human_confirm") {
    if (state.sessionStatus === "confirmed") {
      return null;
    }
    return state.flowKind !== "single_candidate" ||
      state.singleCandidatePhase === "approval" ||
      state.singleCandidatePhase === "evaluate"
      ? null
      : "phase_mismatch";
  }
  // F-20：story/design legacy 流的 AuthorConfirm 阶段本身即人工门（approve 走 HTTP
  // confirm 端点、terminate 走 abandon_human_gate，wave2-f18-report §5）——不再按
  // 非门阶段拦截；sessionStatus=confirmed（HTTP 200 乐观/权威）即视为已收口。
  if (isStoryDesignAuthorConfirm(state)) {
    return state.sessionStatus === "confirmed" ? "closed" : null;
  }
  // A1 方案 A（用户 2026-09-30 裁决，REQ-CFC-05 修订为 amendment-aware）：completed
  // stage 的终态守卫只锁「可无歧义判定为陈旧」的形态。「completed+snapshot」是
  // REQ-GCE-03 amendment 重开的合法载体形态——work_item_plan + single_candidate +
  // 相位 completed + 快照在场（镜像引擎 probe_amendment_gate_context 的前端可见
  // 前置谓词，conversational_gate.rs:165-201；amendment 上下文事实 build_session_state
  // 未暴露，前端不可无歧义判定陈旧）——维持放行，真实陈旧由引擎既有拒绝
  // （INVALID_MESSAGE_FOR_STAGE 系）兜底，本守卫不得改引擎拒绝语义。不构成载体
  // 形态（非 plan/非 SC 流/相位非 completed/无快照凭据的残留 turn）在引擎侧不存在
  // 任何合法重开路径 = 0017 形态的可无歧义陈旧门，锁死并给出原因说明。
  if (state.stage === "completed") {
    const amendmentCarrierShape =
      state.workspaceType === "work_item_plan" &&
      state.flowKind === "single_candidate" &&
      state.singleCandidatePhase === "completed" &&
      state.humanGateSnapshot !== null;
    return amendmentCarrierShape ? null : "terminal_stage";
  }
  if (TERMINAL_GATE_STAGES[state.stage]) {
    return "terminal_stage";
  }
  return state.humanGateSnapshot || state.humanGateTurn ? null : "terminal_stage";
}

/**
 * F-21（v28 监控 0449，v26/v27/v28 三次未修）：终止专属阻断判据。
 * plan 会话（SC 流）除 approval/evaluate 终审门外还会停在 human_confirm——
 * context blocker（prepare 相位，workspace_engine/plan_outline/authoring.rs
 * enter_work_item_plan_context_blocker）/author 连续 validate 失败（generate
 * 相位，decisions.rs enter_human_confirm_for_work_item_plan_author_failure）/
 * 旧会话缺相位字段（null）。矩阵（workspace_ws_handler/protocol.rs HumanConfirm
 * SC 臂）已放行 AbandonHumanGate（59d59760），引擎 close_human_gate 只校验
 * stage+flow_kind 不校验相位——这些形态的 phase_mismatch 不得拦终止
 * （confirm/feedback 维持 gateActionBlockReason 相位纪律，由渲染面分流）。
 */
export function gateTerminateBlockReason(state: WorkspaceWsState): GateActionBlockReason {
  const reason = gateActionBlockReason(state);
  if (
    reason === "phase_mismatch" &&
    state.workspaceType === "work_item_plan" &&
    state.stage === "human_confirm" &&
    state.flowKind === "single_candidate"
  ) {
    return null;
  }
  return reason;
}

export function gateActionBlockCopy(reason: Exclude<GateActionBlockReason, null>): string {
  switch (reason) {
    case "terminal_stage":
      return "已离开人工确认门";
    case "phase_mismatch":
      return "门相位与当前阶段不一致";
    case "closed":
      return "该人工确认门已关闭";
  }
}

export interface GateProjection {
  key: string;
  /** REQ-PCG-01/02：门种类——既有四路均为 "human_gate"（零破坏）。 */
  kind: GateKind;
  turn_id: string | null;
  stage: string;
  flow_kind: WorkspaceWsState["flowKind"];
  status: HumanGateTurnState["status"];
  trigger: WorkItemPlanHumanGateSnapshot["trigger"] | null;
  remaining_budget: number | null;
  findings: WorkItemPlanHumanGateSnapshot["findings"];
  resumable: boolean;
  triage: boolean;
  closed: GateClosureDecision | null;
  closure_stage: string | null;
  opened_at: string;
  turn: HumanGateTurnState | null;
  action_block_reason?: GateActionBlockReason;
  /** 终止专属阻断（F-21）：null 即终止可发；缺席时回退 action_block_reason。 */
  terminate_block_reason?: GateActionBlockReason;
  /**
   * REQ-WSC-18：当前候选 Final Compile 失败事实（store 归一；null=候选
   * 可编译）——门卡据此进「候选编译不过」态（指引/原因/距通过/确认判据
   * 同一份事实派生）。仅 human_gate 分支携带。
   */
  candidate_compile_failure?: WorkItemPlanCandidateCompileFailure | null;
  /**
   * F-31（v37 复验 #2）：author 门可否「确认并评审」（with_review=true）——
   * = reviewerEnabled；仅 story/design author_confirm 分支设置，其余门缺省。
   */
  review_available?: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function latestReviewVerdictEntry(state: WorkspaceWsState): ChatEntry | undefined {
  return state.chatEntries.filter((entry) => entry.type === "review_verdict").at(-1);
}

export function isGateTriage(state: WorkspaceWsState): boolean {
  const metadata = latestReviewVerdictEntry(state)?.metadata;
  if (isRecord(metadata)) {
    if (metadata.review_gate === "user_triage_required") {
      return true;
    }
    if (metadata.verdict === "needs_human") {
      return true;
    }
  }
  return state.pendingReviewerSummary?.verdict === "needs_human";
}

/**
 * REQ-PCG-01/02：两新门的投影形状相同（无 turn/snapshot/预算/findings），只有
 * kind、key 与 opened_at 来源不同——key 恒取 `node:${node_id}`（durable node 身份），
 * 同一门在重复帧/重连重投影下保持同一身份，不重复追加收件箱条目。
 */
function nodeGateProjection(input: {
  state: WorkspaceWsState;
  node: TimelineNode;
  kind: "batch_confirm" | "compile_recovery";
  actionBlockReason: GateActionBlockReason;
  terminateBlockReason: GateActionBlockReason;
}): GateProjection {
  const { state, node } = input;
  return {
    key: `node:${node.node_id}`,
    kind: input.kind,
    turn_id: null,
    stage: state.stage,
    flow_kind: state.flowKind,
    status: "open",
    trigger: null,
    remaining_budget: null,
    findings: [],
    resumable: false,
    // 两新门没有 review verdict 分诊语义（triage 专属 review gate）：恒 false，
    // 不收件箱提级、不把流程行标成 awaiting_triage。
    triage: false,
    closed: state.humanGateClosure?.decision ?? null,
    closure_stage: state.humanGateClosure?.stage ?? null,
    opened_at: node.started_at,
    turn: null,
    action_block_reason: input.actionBlockReason,
    terminate_block_reason: input.terminateBlockReason,
  };
}

// 单一事实源：收件箱门禁条目与 ③ 区门卡都从本函数派生。
// 存在条件 = 「有 store gate 投影」：typed turn / durable snapshot / legacy human_confirm 阶段三者之一。
export function selectGateProjection(state: WorkspaceWsState): GateProjection | null {
  const snapshot = state.humanGateSnapshot;
  const closure = state.humanGateClosure;
  const triage = isGateTriage(state);
  const turn = state.humanGateTurn;
  const actionBlockReason = gateActionBlockReason(state);
  const terminateBlockReason = gateTerminateBlockReason(state);
  // F-42：终态会话（confirmed/terminated）无关门帧也按关门投影——见 terminalGateClosure。
  const terminalClosure = terminalGateClosure(state);

  if (turn) {
    // F-49 A8：残留 turn 的预算是上一轮的读数——「修订成功经 Evaluate 重建门快照」
    // 时快照预算重置为默认值（spec REQ-CG-02；实测 conversational_gate_amendment_
    // real_chain.rs 重建后 manual_repairs_remaining=3），而 turn 按 D8 跨
    // session_state 保留（command_id 去重）。判据：门快照身份变了（= 门载体已换
    // 快照），预算以新快照为准；快照未变（turn 在飞 / 门未重建）维持 turn 值；
    // turn 开出时无快照凭据（undefined）也维持 turn 值（fail-closed）。
    const snapshotSupersedesTurnBudget =
      snapshot !== null &&
      state.snapshotGateIdentity !== null &&
      turn.opened_snapshot_identity != null &&
      state.snapshotGateIdentity !== turn.opened_snapshot_identity;
    return {
      key: gateIdentityFromState(state) ?? turn.turn_id,
      kind: "human_gate",
      turn_id: turn.turn_id,
      stage: state.stage,
      flow_kind: state.flowKind,
      status: turn.status,
      trigger: snapshot?.trigger ?? null,
      remaining_budget: snapshotSupersedesTurnBudget
        ? snapshot.manual_repairs_remaining
        : turn.remaining_budget,
      findings: snapshot?.findings ?? [],
      resumable: snapshot?.resumable ?? false,
      triage,
      closed: closure?.decision ?? terminalClosure,
      closure_stage: closure?.stage ?? null,
      opened_at: turn.opened_at,
      turn,
      action_block_reason: actionBlockReason,
      terminate_block_reason: terminateBlockReason,
      candidate_compile_failure: state.candidateCompileFailure ?? null,
    };
  }

  const batchNode = batchConfirmGateNode(state);
  const recoveryNode = compileRecoveryGateNode(state);
  // REQ-PCG-03/F-30：confirmed/terminated 已由服务端持久化——残留的两新门节点
  // 与迟到帧都不得复活待处理门（含下面 human_confirm 兜底分支）。
  if (
    (batchNode !== null || recoveryNode !== null) &&
    TERMINAL_SESSION_STATUSES[state.sessionStatus ?? ""] === true
  ) {
    return null;
  }

  if (batchNode !== null) {
    return nodeGateProjection({
      state,
      node: batchNode,
      kind: "batch_confirm",
      actionBlockReason,
      terminateBlockReason,
    });
  }

  if (recoveryNode !== null) {
    return nodeGateProjection({
      state,
      node: recoveryNode,
      kind: "compile_recovery",
      actionBlockReason,
      terminateBlockReason,
    });
  }

  if (snapshot) {
    return {
      key: gateIdentityFromState(state) ?? `snapshot:${state.stage}`,
      kind: "human_gate",
      turn_id: null,
      stage: state.stage,
      flow_kind: state.flowKind,
      status: "open",
      trigger: snapshot.trigger,
      remaining_budget: snapshot.manual_repairs_remaining,
      findings: snapshot.findings,
      resumable: snapshot.resumable,
      triage,
      closed: closure?.decision ?? terminalClosure,
      closure_stage: closure?.stage ?? null,
      opened_at: state.snapshotGateOpenedAt ?? "",
      turn: null,
      action_block_reason: actionBlockReason,
      terminate_block_reason: terminateBlockReason,
      candidate_compile_failure: state.candidateCompileFailure ?? null,
    };
  }
  // F-20：story/design legacy 流 author_confirm 即人工门（无 typed turn/durable
  // snapshot），以 stage 前缀投影——收件箱门条/对话流门卡/审计 gateId 同源派生。
  // confirmed（HTTP confirm 200 乐观/权威）即关门（F-42 起判据统一走
  // terminalGateClosure：confirmed/terminated 都关门），收件箱不再挂等待项。
  if (isStoryDesignAuthorConfirm(state)) {
    return {
      key: `stage:${state.stage}`,
      kind: "human_gate",
      turn_id: null,
      stage: state.stage,
      flow_kind: state.flowKind,
      status: "open",
      trigger: null,
      remaining_budget: null,
      findings: [],
      resumable: false,
      triage,
      closed: closure?.decision ?? terminalClosure,
      closure_stage: closure?.stage ?? (terminalClosure !== null ? state.stage : null),
      opened_at: "",
      turn: null,
      action_block_reason: actionBlockReason,
      terminate_block_reason: terminateBlockReason,
      review_available: state.reviewerEnabled,
    };
  }


  if (state.stage !== "human_confirm") {
    return null;
  }

  return {
    key: gateIdentityFromState(state) ?? `stage:${state.stage}`,
    kind: "human_gate",
    turn_id: null,
    stage: state.stage,
    flow_kind: state.flowKind,
    status: "open",
    trigger: null,
    remaining_budget: null,
    findings: [],
    resumable: false,
    triage,
    closed: closure?.decision ?? terminalClosure,
    closure_stage: closure?.stage ?? null,
    opened_at: "",
    turn: null,
    action_block_reason: actionBlockReason,
    terminate_block_reason: terminateBlockReason,
  };
}
