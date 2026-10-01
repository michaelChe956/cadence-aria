import type { TimelineNode } from "./workspace-ws-store-types";

// 从 workspace-cockpit-projection.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 执行流（自动执行流窄栏）的行状态/时长/拓扑投影。
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
