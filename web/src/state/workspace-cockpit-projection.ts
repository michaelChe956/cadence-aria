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

  if (turn) {
    return {
      key: turn.turn_id,
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
    };
  }

  if (snapshot) {
    return {
      key: `snapshot:${state.stage}`,
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
      opened_at: "",
      turn: null,
    };
  }

  if (state.stage !== "human_confirm") {
    return null;
  }

  return {
    key: `legacy:${state.stage}`,
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
