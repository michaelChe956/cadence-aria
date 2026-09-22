import { gateIdentityFromState } from "./cockpit-action-routing";
import type { WorkItemPlanHumanGateSnapshot } from "../api/types";
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
  if (state.humanGateClosure?.decision) {
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
  if (state.stage === "completed" && state.humanGateSnapshot) {
    return null;
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

  if (turn) {
    return {
      key: gateIdentityFromState(state) ?? turn.turn_id,
      kind: "human_gate",
      turn_id: turn.turn_id,
      stage: state.stage,
      flow_kind: state.flowKind,
      status: turn.status,
      trigger: snapshot?.trigger ?? null,
      remaining_budget: turn.remaining_budget,
      findings: snapshot?.findings ?? [],
      resumable: snapshot?.resumable ?? false,
      triage,
      closed: closure?.decision ?? null,
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
      closed: closure?.decision ?? null,
      closure_stage: closure?.stage ?? null,
      opened_at: state.snapshotGateOpenedAt ?? "",
      turn: null,
      action_block_reason: actionBlockReason,
      terminate_block_reason: terminateBlockReason,
    };
  }
  // F-20：story/design legacy 流 author_confirm 即人工门（无 typed turn/durable
  // snapshot），以 stage 前缀投影——收件箱门条/对话流门卡/审计 gateId 同源派生。
  // confirmed（HTTP confirm 200 乐观/权威）即关门，收件箱不再挂等待项。
  if (isStoryDesignAuthorConfirm(state)) {
    const confirmed = state.sessionStatus === "confirmed";
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
      closed: confirmed ? "confirm" : (closure?.decision ?? null),
      closure_stage: confirmed ? state.stage : (closure?.stage ?? null),
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
    closed: closure?.decision ?? null,
    closure_stage: closure?.stage ?? null,
    opened_at: "",
    turn: null,
    action_block_reason: actionBlockReason,
    terminate_block_reason: terminateBlockReason,
  };
}

export type CockpitInboxKind = "gate" | "stopped" | "hard_error";

export interface CockpitInboxItem {
  id: string;
  kind: CockpitInboxKind;
  severity: 1 | 2 | 3;
  title: string;
  summary: string;
  triage: boolean;
  source: "gate" | "session_status" | "protocol_error" | "engine_error" | "advance";
  createdAt: string | null;
  gate: GateProjection | null;
  inlineError: { code: string; message: string } | null;
  /** protocol_error 来源条目的机器码（如 STALE_DRIVER_LEASE）；其余来源缺省。 */
  protocolErrorCode?: string | null;
}

/** 裸 driver 抢走租约后，本连接写操作被拒的协议码（F-11 恢复入口依据）。 */
export const STALE_DRIVER_LEASE_CODE = "STALE_DRIVER_LEASE";

export function isStaleDriverLeaseItem(item: CockpitInboxItem): boolean {
  return item.source === "protocol_error" && item.protocolErrorCode === STALE_DRIVER_LEASE_CODE;
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
  return gate.triage ? "门禁等待（需分诊）" : "门禁等待";
}

function gateInboxHeadline(gate: GateProjection): string {
  if (gate.kind === "batch_confirm") {
    return "等待整组 Work Item Draft 确认";
  }
  if (gate.kind === "compile_recovery") {
    return "Final Compile 中断，等待恢复动作";
  }
  return gate.trigger ? GATE_TRIGGER_LABELS[gate.trigger] : "等待人工确认";
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
    });
  }

  if (state.protocolError) {
    items.push({
      id: `hard_error:protocol:${state.protocolError.code}`,
      kind: "hard_error",
      severity: 3,
      title: `协议错误 ${state.protocolError.code}`,
      summary: state.protocolError.message,
      triage: false,
      source: "protocol_error",
      createdAt: null,
      gate: null,
      inlineError: null,
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
    });
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
