import { gateIdentityFromState } from "./cockpit-action-routing";
import { protocolErrorCopy, STALE_DRIVER_LEASE_CODE } from "./protocol-error-copy";
import type {
  C1WaitingItem,
  ChoiceQuestion,
  CodingFinalConfirmInfoItem,
  LogicalCodebaseBootstrapNoticeDto,
  LogicalCodebaseBootstrapProjection,
  PlanConfirmedInfoItem,
  WorkItemPlanHumanGateSnapshot,
} from "../api/types";
import type { CodingAttemptAddress } from "../api/types/coding";
import type { ChatEntry } from "./chat-entries";
import type {
  GateClosureDecision,
  HumanGateTurnState,
  TimelineNode,
  WorkspaceWsState,
} from "./workspace-ws-store-types";

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
function compileRecoveryGateNode(
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

export type CockpitInboxKind =
  | "gate"
  | "stopped"
  | "hard_error"
  | "choice"
  | "info"
  | "c1_recovery"
  | "sc_failed"
  | "lc_bootstrap";

/**
 * P0 1.3（REQ-WIGA-05）Task 11：驾驶舱 choice 就地作答投影——workspace 侧
 * 来自 session_state `pending_choice_requests`（含完整逐题结构），coding 侧
 * 来自 attempt snapshot 的 open choice gate（页面按需拉取）。status 是本地
 * 命令状态与服务端 pending 列表的合并视图：open=可作答；
 * submitting/resolving=202 已受理保卡复查；delivered=回执已送达（卡片移除）；
 * expired=410 旧 run 失效（不路由到新 run）。
 */
export interface ChoiceInboxProjection {
  sessionId: string | null;
  choiceId: string;
  prompt: string;
  expectedRunId: string | null;
  questions: ChoiceQuestion[];
  allowMultiple: boolean;
  allowFreeText: boolean;
  source: "workspace" | "coding";
  attemptAddress?: CodingAttemptAddress;
  status: "open" | "submitting" | "resolving" | "delivered" | "expired";
}

/**
 * P1 WIGA Task 9（REQ-WIGA-07）：durable plan 确认 info 的只读投影——不进
 * countedInbox/批量/危险操作，仅供「进度信息」分区显示与一次提醒。
 */
export interface PlanConfirmedInfoProjection {
  key: string;
  planId: string;
  sessionId: string;
  occurredAt: string;
  title: string;
}

/**
 * P2 WIGA Task 9（REQ-WIGA-07/R5）：durable coding FinalConfirm 等待/已确认
 * 信息的只读投影——不进 countedInbox/批量/危险操作，仅供「进度信息」分区
 * 显示、一次提醒与 Coding Workspace 下钻。
 */
export interface CodingFinalConfirmInfoProjection {
  projectId: string;
  issueId: string;
  planId: string;
  attemptId: string;
  key: string;
  occurredAt: string;
  finalConfirmed: boolean;
}

export interface CockpitInboxItem {
  id: string;
  kind: CockpitInboxKind;
  severity: 1 | 2 | 3;
  title: string;
  summary: string;
  triage: boolean;
  source:
    | "gate"
    | "session_status"
    | "protocol_error"
    | "engine_error"
    | "advance"
    | "choice"
    | "plan_confirmed_info"
    | "coding_final_confirm_info"
    | "c1_waiting"
    | "sc_failed"
    | "logical_codebase_bootstrap",
  createdAt: string | null;
  gate: GateProjection | null;
  inlineError: { code: string; message: string } | null;
  /** REQ-WIGA-05 Task 11：kind="choice" 时的作答投影；其余 kind 恒 null。 */
  choice: ChoiceInboxProjection | null;
  /** protocol_error 来源条目的机器码（如 STALE_DRIVER_LEASE）；其余来源缺省。 */
  protocolErrorCode?: string | null;
  /** REQ-WIGA-07 Task 9：kind="info" 时的 plan 确认投影；其余 kind 缺省。 */
  planInfo?: PlanConfirmedInfoProjection | null;
  /** REQ-WIGA-07/R5 Task 9：kind="info" 时的 coding FinalConfirm 投影；其余 kind 缺省。 */
  codingInfo?: CodingFinalConfirmInfoProjection | null;
  /** P3（REQ-WIGA-07）：近期完成事实的稳定身份（JSON [scope,kind,key]）；
   * 仅 recent 目录投影填写，提示去重以此为准（不取易碰撞的 item.id）。 */
  completionIdentity?: string;
  /** P2 GAP-E/G（Task 0.1）：kind="sc_failed" 时的人工显式重驱投影。 */
  scFailure?: { failedNodeId: string; phase: "failed" } | null;
  /** C1（enrollment-recovery-surface Task 9）：kind="c1_recovery" 时的
   * durable 恢复等待项投影（reason/身份/可用动作/下一阶段）。 */
  c1Info?: C1WaitingProjection | null;
  /** C4（LC 冷启动加固 Task 9）：kind="lc_bootstrap" 时的 durable 冷启动
   * 等待/失败通知投影（notice key/step/object/原因/副作用/动作/下一步）；
   * 不进入错误计数，动作统一走 bootstrap action API。 */
  bootstrapInfo?: LogicalCodebaseBootstrapInfoProjection | null;
}

/** C4 Task 9：LC 冷启动通知的收件箱投影（服务端
 * LogicalCodebaseBootstrapNotice 的只读镜像；字段一一对应，前端不另造
 * 事实，按钮生成稳定 command id 后经统一 action API 出站）。 */
export interface LogicalCodebaseBootstrapInfoProjection {
  projectId: string;
  logicalCodebaseId: string;
  noticeKey: string;
  step: string;
  objectId: string;
  reasonCode: string;
  summary: string;
  externalSideEffect: string;
  allowedActions: readonly string[];
  nextStep: string | null;
  membershipRevision: number | null;
}

/** C2 Task 12：等待项操作上下文的只读镜像（服务端 WaitingItemAction）。 */
export interface WaitingItemActionProjection {
  action: string;
  commandId: string;
  expectedVersion: number;
}

/** C1 Task 9：durable 恢复等待项的收件箱投影（服务端 C1WaitingItem 的
 * 只读镜像；动作按 actions 名由 facade 触发对应 REST，前端不判定成功）。
 * C2 Task 12 additive：expectedVersion／actionContext（C2 kind 同通道）。
 * C5 Task 6 additive：project 级条目无 issue（issueId 为 null），身份用
 * 后端 operation_id（id 即后端稳定 id，不从展示字符串反解析）。 */
export interface C1WaitingProjection {
  projectId: string;
  issueId: string | null;
  itemId: string;
  kind: string;
  reason: string;
  completedSteps: readonly string[];
  targetLabel: string;
  planId: string | null;
  sessionId: string | null;
  attemptId: string | null;
  gateId: string | null;
  possibleSideEffect: string | null;
  actions: readonly string[];
  nextPhase: string | null;
  expectedVersion: number | null;
  actionContext: readonly WaitingItemActionProjection[];
  // C5 Task 6 additive：repository 初始化失败等待项的操作身份与诊断。
  operationId: string | null;
  diagnostics: RepositoryInitFailureDiagnosticsProjection | null;
}

/** C5 Task 6：repository 初始化失败诊断的只读镜像（结构化原因/步骤/路径）。 */
export interface RepositoryInitFailureDiagnosticsProjection {
  failedStep: string;
  reasonCode: string;
  provider: string | null;
  stderrSummary: string | null;
  changedPaths: readonly string[];
  retryable: boolean;
}

/** C1/C2 等待项标题（按 kind 固定文案；身份字段进摘要行）。 */
const C1_WAITING_KIND_TITLES: Record<string, string> = {
  candidate_recovery: "候选门快照待恢复",
  lease_wait: "租约活跃，自动链等待中",
  lease_takeover: "死亡租约待确认接管",
  lease_unknown: "租约活性未知，停等人工",
  advance_retry_failed: "Failed advance 待显式重试",
  intent_blocked: "计划意图停等修订",
  generation_history: "存在旧代绑定（只读可查）",
  // C5 Task 6：project 级 repository 初始化失败等待项（网关恢复后可续）。
  repository_initialization_failed: "仓库初始化失败，等待恢复",
  // C2 Task 12：coding 链十类等待项。
  coding_completion_unconfirmed: "Coding 完成状态待确认",
  coding_already_running: "Coding 已在运行，请等待",
  coding_takeover_required: "Coding 死亡租约待确认接管",
  coding_lease_unknown: "Coding 租约活性未知，停等人工",
  coding_restart_available: "Coding 终态可显式重启",
  reviewer_configuration_missing: "Reviewer 配置缺失，停等补齐",
  verification_triage: "验证处理待人工裁决",
  policy_verification: "政策核验停等（fail-closed）",
  instruction_claim_interrupted: "返修指令消费中断，待同认领重放",
  large_candidate_blocked: "大候选超预算，停等分段返修",
};

export function c1TargetLabel(target: C1WaitingItem["target"]): string {
  if (!target) {
    return "target 未知";
  }
  return target.kind === "single_repository"
    ? `单仓 ${target.repository_id}`
    : `逻辑代码库 ${target.logical_codebase_id}/${target.logical_repository_id}`;
}

/** durable C1WaitingItem → 收件箱条目（不进 countedInbox；动作走 C1 REST）。
 * C5 Task 6：project 级条目不传 issueId——id 直接采用后端稳定 id
 *（c1:project:{project_id}:repository_init:{operation_id}），不拼接
 * undefined issueId；issue 级条目维持 `c1:{issueId}:{item.id}` 前缀。 */
export function c1WaitingItem(
  item: C1WaitingItem,
  projectId: string,
  issueId?: string,
): CockpitInboxItem {
  const identityParts = [
    item.plan_id ? `plan ${item.plan_id}` : null,
    item.session_id ? `session ${item.session_id}` : null,
    item.attempt_id ? `attempt ${item.attempt_id}` : null,
    item.gate_id ? `gate ${item.gate_id}` : null,
    c1TargetLabel(item.target),
  ].filter((part): part is string => Boolean(part));
  const summaryParts = [
    item.reason,
    identityParts.length > 0 ? identityParts.join(" · ") : null,
    item.possible_side_effect ? `可能副作用：${item.possible_side_effect}` : null,
    item.next_phase ? `下一阶段：${item.next_phase}` : null,
  ].filter((part): part is string => Boolean(part));
  return {
    id: issueId !== undefined ? `c1:${issueId}:${item.id}` : item.id,
    kind: "c1_recovery",
    severity: item.actions.length > 0 ? 2 : 3,
    title: C1_WAITING_KIND_TITLES[item.kind] ?? "C1 恢复等待项",
    summary: summaryParts.join(" · "),
    triage: false,
    source: "c1_waiting",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    c1Info: {
      projectId,
      issueId: issueId ?? null,
      itemId: item.id,
      kind: item.kind,
      reason: item.reason,
      completedSteps: item.completed_steps,
      targetLabel: c1TargetLabel(item.target),
      planId: item.plan_id ?? null,
      sessionId: item.session_id ?? null,
      attemptId: item.attempt_id ?? null,
      gateId: item.gate_id ?? null,
      possibleSideEffect: item.possible_side_effect ?? null,
      actions: item.actions,
      nextPhase: item.next_phase ?? null,
      // C2 Task 12 additive：expected 版本与操作上下文（旧响应缺失按空）。
      expectedVersion: item.expected_version ?? null,
      actionContext: (item.action_context ?? []).map((action) => ({
        action: action.action,
        commandId: action.command_id,
        expectedVersion: action.expected_version,
      })),
      // C5 Task 6 additive：project 级条目操作身份（后端稳定 operation_id）。
      operationId: item.operation_id ?? null,
      diagnostics: item.diagnostics
        ? {
            failedStep: item.diagnostics.failed_step,
            reasonCode: item.diagnostics.reason_code,
            provider: item.diagnostics.provider ?? null,
            stderrSummary: item.diagnostics.stderr_summary ?? null,
            changedPaths: item.diagnostics.changed_paths ?? [],
            retryable: item.diagnostics.retryable ?? false,
          }
        : null,
    },
  };
}

/** C4 Task 9 步骤名的固定中文文案（与服务端 as_str 一一对应）。 */
const LC_BOOTSTRAP_STEP_LABELS: Record<string, string> = {
  identity: "身份",
  manifest_checkout: "清单/检出",
  rules_policy: "规则/政策",
  member_index: "成员索引",
  aggregate_index_active: "聚合索引激活",
};

/** C4 Task 9 步骤失败/等待原因的固定标题（未知 reason 回落到通用文案）。 */
const LC_BOOTSTRAP_REASON_TITLES: Record<string, string> = {
  identity_migration_failed: "逻辑代码库身份迁移失败，等待人工修复",
  aggregate_initialization_failed: "成员索引构建失败，等待显式重试",
  aggregate_index_failed: "聚合索引首建失败，等待显式重试",
  bootstrap_waiting_for_human: "逻辑代码库冷启动等待人工动作",
  member_rules_missing: "成员规则材料缺失，provider 保持零启动",
};

/** C4 Task 9：durable bootstrap notice → 只读收件箱条目（不进错误计数；
 * id 以稳定 notice key 派生，同 key 只保留一条，重放不重复计数）。 */
export function logicalCodebaseBootstrapItem(
  notice: LogicalCodebaseBootstrapNoticeDto,
  projection: Pick<
    LogicalCodebaseBootstrapProjection,
    "project_id" | "logical_codebase_id" | "membership_revision"
  >,
): CockpitInboxItem {
  const stepLabel = LC_BOOTSTRAP_STEP_LABELS[notice.step] ?? notice.step;
  const summaryParts = [
    notice.summary,
    `步骤：${stepLabel} · 对象 ${notice.object_id}`,
    notice.external_side_effect && notice.external_side_effect !== "none"
      ? `可能外部副作用：${notice.external_side_effect}`
      : null,
    notice.next_step
      ? `下一步：${LC_BOOTSTRAP_STEP_LABELS[notice.next_step] ?? notice.next_step}`
      : null,
  ].filter((part): part is string => Boolean(part));
  return {
    id: `lc-bootstrap:${projection.project_id}:${projection.logical_codebase_id}:${notice.key}`,
    kind: "lc_bootstrap",
    severity: 2,
    title: LC_BOOTSTRAP_REASON_TITLES[notice.reason_code]
      ?? `逻辑代码库冷启动等待：${notice.reason_code}`,
    summary: summaryParts.join(" · "),
    triage: false,
    source: "logical_codebase_bootstrap",
    createdAt: notice.created_at || null,
    gate: null,
    inlineError: null,
    choice: null,
    bootstrapInfo: {
      projectId: projection.project_id,
      logicalCodebaseId: projection.logical_codebase_id,
      noticeKey: notice.key,
      step: notice.step,
      objectId: notice.object_id,
      reasonCode: notice.reason_code,
      summary: notice.summary,
      externalSideEffect: notice.external_side_effect,
      allowedActions: notice.allowed_actions,
      nextStep: notice.next_step ?? null,
      membershipRevision: projection.membership_revision ?? null,
    },
  };
}

/** 裸 driver 抢走租约后的协议码——常量源头在 protocol-error-copy（F-50）。 */
export { STALE_DRIVER_LEASE_CODE } from "./protocol-error-copy";

export function isStaleDriverLeaseItem(item: CockpitInboxItem): boolean {
  return item.source === "protocol_error" && item.protocolErrorCode === STALE_DRIVER_LEASE_CODE;
}

/** REQ-WIGA-07：durable plan_confirmed_info → 只读 info 收件箱条目。 */
export function planConfirmedInfoItem(info: PlanConfirmedInfoItem): CockpitInboxItem {
  return {
    id: `${info.session_id}:info:${info.key}`,
    kind: "info",
    severity: 1,
    title: info.title,
    summary: `plan ${info.plan_id} 已确认 · 确认于 ${info.occurred_at}`,
    triage: false,
    source: "plan_confirmed_info",
    createdAt: info.occurred_at,
    gate: null,
    inlineError: null,
    choice: null,
    planInfo: {
      key: info.key,
      planId: info.plan_id,
      sessionId: info.session_id,
      occurredAt: info.occurred_at,
      title: info.title,
    },
  };
}

/** REQ-WIGA-07/R5：durable coding_final_confirm_info → 只读 info 收件箱条目。 */
export function codingFinalConfirmInfoItem(
  info: CodingFinalConfirmInfoItem,
): CockpitInboxItem {
  return {
    id: `${info.issue_id}:info:${info.key}`,
    kind: "info",
    severity: 1,
    title: info.title,
    summary: info.final_confirmed
      ? `编码执行已完成人工最终确认 · ${info.occurred_at}`
      : `等待人工最终确认 · ${info.occurred_at}`,
    triage: false,
    source: "coding_final_confirm_info",
    createdAt: info.occurred_at,
    gate: null,
    inlineError: null,
    choice: null,
    codingInfo: {
      projectId: info.project_id,
      issueId: info.issue_id,
      planId: info.plan_id,
      attemptId: info.attempt_id,
      key: info.key,
      occurredAt: info.occurred_at,
      finalConfirmed: info.final_confirmed,
    },
  };
}

/** 收件箱条目 id（`${sessionId}:gate:${key}` 等形态）的会话归属；无会话前缀返回 null。 */
export function cockpitInboxItemSessionId(itemId: string): string | null {
  const separator = itemId.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const sessionId = itemId.slice(0, separator);
  return sessionId === "gate" || sessionId === "hard_error" || sessionId === "stopped"
    ? null
    : sessionId;
}

/**
 * REQ-PCG-01/02：门条标题按门种类给出动作语义；两新门不复用 legacy 逐段决策文案。
 */
function gateInboxTitle(gate: GateProjection): string {
  if (gate.kind === "batch_confirm") {
    return "确认整组 Work Item Draft";
  }
  if (gate.kind === "compile_recovery") {
    return "Final Compile 恢复";
  }
  // F-50 fix round 标题去重：triage 短语只保留主区门卡卡头一处——抽屉门条目
  // 与非 triage 门同题；triage 细节由主区门卡头+原因行承载。
  return "需要人工确认";
}

function gateInboxHeadline(gate: GateProjection): string | null {
  if (gate.kind === "batch_confirm") {
    return "等待整组 Work Item Draft 确认";
  }
  if (gate.kind === "compile_recovery") {
    return "Final Compile 中断，等待恢复动作";
  }
  // F-50 §3.5：触发原因独立成「原因：…」行（与门卡 gate-why 同构），
  // 不再用标题重复人工介入语义；无 trigger 即无独立事实，返回 null 让位
  // （parts 已 filter），不以「等待人工确认」与标题「需要人工确认」近义叠行
  // （fix1，k3 P2——story/design author_confirm 主流程 trigger 恒 null）。
  return gate.trigger ? `原因：${GATE_TRIGGER_LABELS[gate.trigger]}` : null;
}

export function selectCockpitInbox(state: WorkspaceWsState): CockpitInboxItem[] {
  const items: CockpitInboxItem[] = [];
  const gate = selectGateProjection(state);

  if (gate && gate.closed === null) {
    const findingsCount = gate.findings.length;
    // REQ-PCG-02：recovery 门必须展示中断原因（引擎写在 recovery timeline node 的
    // summary）；原因未同步时只读诊断文案，不以默认动作猜测。
    const recoveryReason =
      gate.kind === "compile_recovery"
        ? (compileRecoveryGateNode(state)?.summary ?? null)
        : null;
    const parts = [
      gateInboxHeadline(gate),
      gate.kind === "compile_recovery"
        ? (recoveryReason ?? "中断原因未同步（只读诊断）")
        : null,
      findingsCount > 0 ? `findings ${findingsCount} 条` : null,
      gate.remaining_budget !== null ? `剩余修复轮次 ${gate.remaining_budget}` : null,
      gate.turn?.status === "failed" && gate.turn.failure_message
        ? `失败：${gate.turn.failure_class ?? "unknown"} · ${gate.turn.failure_message}`
        : null,
      gate.turn?.status === "busy" ? "引擎在处理中" : null,
    ].filter((part): part is string => Boolean(part));

    items.push({
      id: `gate:${gate.key}`,
      kind: "gate",
      severity: gate.triage ? 2 : 1,
      title: gateInboxTitle(gate),
      summary: parts.join(" · "),
      triage: gate.triage,
      source: "gate",
      createdAt: gate.opened_at || null,
      gate,
      inlineError: gate.turn?.inlineError ?? null,
      choice: null,
    });
  }

  if (state.sessionStatus === "stopped_needs_human") {
    items.push({
      id: `stopped:${state.sessionId ?? "session"}`,
      kind: "stopped",
      severity: 2,
      title: "会话停在停点",
      // L4（最小版）：停点原因从 session 状态字段取，后缀为静态接管文案；
      // 完整停点指纹（已完成原因 + 已完成进度 + 失败指纹）随 1b 的 takeover 接线（REQ-UI37-08，tasks.md 2.9）深化。
      summary: `停点原因：${state.sessionStatus} · 等待人工接管后继续`,
      triage: false,
      source: "session_status",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
    });
  }

  if (state.protocolError) {
    items.push({
      id: `hard_error:protocol:${state.protocolError.code}`,
      kind: "hard_error",
      severity: 3,
      // F-50 裁决 6：中文主显 lead（错误码由 protocolErrorCode 走 mono 副行，
      // 英文原文留在 summary 由错误条折叠呈现）。
      title: protocolErrorCopy(state.protocolError.code).lead,
      summary: state.protocolError.message,
      triage: false,
      source: "protocol_error",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
      protocolErrorCode: state.protocolError.code,
    });
  }

  if (state.error) {
    items.push({
      id: "hard_error:error",
      kind: "hard_error",
      severity: 3,
      title: "引擎错误",
      summary: state.error,
      triage: false,
      source: "engine_error",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
    });
  }

  for (const command of Object.values(state.advanceCommands)) {
    if (command.status !== "rejected" || command.code === null || isAdvanceReplayCode(command.code)) {
      continue;
    }
    items.push({
      id: `hard_error:advance:${command.command_id}`,
      kind: "hard_error",
      severity: 3,
      title: "推进被拒",
      summary: `${command.code} · ${command.reason ?? ""}`.trim(),
      triage: false,
      source: "advance",
      createdAt: null,
      gate: null,
      inlineError: command.inlineError,
      choice: null,
    });
  }

  // P0 1.3（REQ-WIGA-05）Task 11：pending choice 逐条一张卡（不可批量选择，
  // isSelectableGate 只认 human_gate）。expected_run_id 缺失（旧投影/无活跃
  // run）不猜 run——卡面提示刷新后作答，不给提交面。
  for (const request of state.pendingChoiceRequests ?? []) {
    items.push({
      id: `choice:${request.id}`,
      kind: "choice",
      severity: 2,
      title: "选择请求待作答",
      summary: request.prompt,
      triage: false,
      source: "choice",
      createdAt:
        request.created_at_ms !== null ? new Date(request.created_at_ms).toISOString() : null,
      gate: null,
      inlineError: null,
      choice: {
        sessionId: state.sessionId,
        choiceId: request.id,
        prompt: request.prompt,
        expectedRunId: request.expected_run_id,
        questions: request.questions,
        allowMultiple: request.allow_multiple,
        allowFreeText: request.allow_free_text,
        source: "workspace",
        status: "open",
      },
    });
  }

  // P2 GAP-E/G（Task 0.1）：durable Failed 的 SingleCandidate 现场投影——
  // 最新 Failed timeline 节点 + phase=failed；只读事实 + 人工显式重驱动作，
  // 绝不自动重试。非 Failed 相位/非 SC flow 不投影。
  if (
    state.flowKind === "single_candidate" &&
    state.singleCandidatePhase === "failed" &&
    state.sessionStatus === "failed"
  ) {
    const failedNode = [...state.timelineNodes]
      .filter((node) => node.status === "failed")
      .sort((left, right) => right.started_at.localeCompare(left.started_at))[0];
    if (failedNode) {
      items.push({
        id: `sc_failed:${failedNode.node_id}`,
        kind: "sc_failed",
        severity: 2,
        title: "单候选评审运行失败",
        summary: `${failedNode.summary ?? "评审运行失败"} · 等待人工显式重驱`,
        triage: false,
        source: "sc_failed",
        createdAt: failedNode.started_at ?? null,
        gate: null,
        inlineError: null,
        choice: null,
        scFailure: { failedNodeId: failedNode.node_id, phase: "failed" },
      });
    }
  }

  return items.sort((left, right) => {
    if (left.severity !== right.severity) {
      return right.severity - left.severity;
    }
    const leftCreated = left.createdAt ?? "";
    const rightCreated = right.createdAt ?? "";
    if (leftCreated !== rightCreated) {
      return rightCreated.localeCompare(leftCreated);
    }
    return left.id.localeCompare(right.id);
  });
}

export type CockpitFlowState =
  | "running"
  | "pending"
  | "blocked"
  | "done"
  | "failed"
  | "awaiting_triage";

export interface CockpitFlowRow {
  node_id: string;
  title: string;
  state: CockpitFlowState;
  index: number;
  total: number;
  elapsed_ms: number;
  // REQ-UI37-18「长时间无事件」的静默量：自节点最近一次引擎事件（`last_event_at`，
  // 缺省回退 `started_at`）起的毫秒数，与 `elapsed_ms`（节点起点起算）区分开。
  idle_ms: number;
  started_at: string;
  // 只读投影：行数据由 store 派生，消费方只读不写；只读元素类型同时接纳 `as const` 字面量。
  topology: readonly CockpitFlowState[];
}

export function cockpitFlowState(node: TimelineNode, awaitingTriage: boolean): CockpitFlowState {
  switch (node.status) {
    case "active":
      return awaitingTriage ? "awaiting_triage" : "running";
    case "paused":
      return "blocked";
    case "failed":
      return "failed";
    case "skipped":
      return "pending";
    default:
      return "done";
  }
}

export function flowElapsedMs(node: TimelineNode, nowMs: number): number {
  if (typeof node.duration_ms === "number" && node.duration_ms >= 0) {
    return node.duration_ms;
  }
  const started = Date.parse(node.started_at);
  if (Number.isNaN(started)) {
    return 0;
  }
  // #4：终态节点缺 completed_at/duration_ms（引擎 create_timeline_node 直接以终态
  // status 落「流程完成」标记节点，不带结束字段）时，不得拿墙钟续算——冻结为
  // 零时长标记，对齐 append_completed_timeline_event 的 duration_ms=0 口径。
  if (
    node.completed_at == null &&
    (node.status === "completed" || node.status === "failed" || node.status === "skipped")
  ) {
    return 0;
  }
  const ended = node.completed_at ? Date.parse(node.completed_at) : nowMs;
  if (Number.isNaN(ended)) {
    return 0;
  }
  return Math.max(0, ended - started);
}

export function formatFlowElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return `${totalMinutes}m${totalSeconds % 60}s`;
  }
  return `${Math.floor(totalMinutes / 60)}h${totalMinutes % 60}m`;
}

export function topologyTokenName(state: CockpitFlowState): string {
  return state === "awaiting_triage" ? "awaiting-triage" : state;
}

export function selectCockpitFlow(state: WorkspaceWsState, nowMs = Date.now()): CockpitFlowRow[] {
  const total = state.timelineNodes.length;
  // 与收件箱同源：awaiting_triage 是「开放 triage 门」的投影态，门禁关闭后必须回到 running。
  const gate = selectGateProjection(state);
  const awaitingTriage = gate !== null && gate.closed === null && gate.triage;
  const topology = state.timelineNodes.map((node) =>
    cockpitFlowState(node, awaitingTriage && node.node_id === state.activeNodeId),
  );

  return state.timelineNodes.map((node, index) => {
    // REQ-UI37-18：静默量取自最近一次引擎事件；节点没有 `last_event_at`
    // （老数据 / session_state 重建快照）时回退 `started_at`，与既有行为一致。
    const lastEventAt = Date.parse(node.last_event_at ?? node.started_at);
    return {
      node_id: node.node_id,
      title: node.title,
      state: topology[index] ?? "pending",
      index: index + 1,
      total,
      elapsed_ms: flowElapsedMs(node, nowMs),
      idle_ms: Number.isNaN(lastEventAt) ? 0 : Math.max(0, nowMs - lastEventAt),
      started_at: node.started_at,
      topology,
    };
  });
}
