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

  if (snapshot) {
    return {
      key: gateIdentityFromState(state) ?? `snapshot:${state.stage}`,
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

export function selectCockpitInbox(state: WorkspaceWsState): CockpitInboxItem[] {
  const items: CockpitInboxItem[] = [];
  const gate = selectGateProjection(state);

  if (gate && gate.closed === null) {
    const findingsCount = gate.findings.length;
    const parts = [
      gate.trigger ? GATE_TRIGGER_LABELS[gate.trigger] : "等待人工确认",
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
      title: gate.triage ? "门禁等待（需分诊）" : "门禁等待",
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
