import type { ChatEntry } from "./chat-entries";
import { REVISION_INTERRUPTED_SENTENCE } from "./gate-prompt-copy";
import { terminalGateClosure } from "./workspace-cockpit-gate-projection";
import type { TimelineNode, WorkspaceWsState } from "./workspace-ws-store-types";

// REQ-WSC-27 / REQ-CG-10（gate-card-resolved-and-inflight-state）：门卡的「在途态」
// 与「被中断修订证据」两个只读派生。均不新增 store 字段、不新增 wire 字段。

/** 门内 provider run 在途的两种形态：作者修订中 / 复评中。 */
export type GateInflightPhase = "revising" | "reviewing";

/**
 * 被中断修订节点：`status=aborted` 且 summary 含明说句。summary 由后端
 * `revision_node_closure(Cancelled)` 写入（durable，刷新 / 重连后仍在）。
 * 普通中止（summary「运行已中止」）不命中——不误加修订句。
 */
export function isInterruptedRevisionNode(node: TimelineNode): boolean {
  return (
    node.status === "aborted" &&
    typeof node.summary === "string" &&
    node.summary.includes(REVISION_INTERRUPTED_SENTENCE)
  );
}

export function hasInterruptedRevisionEvidence(nodes: TimelineNode[]): boolean {
  return nodes.some(isInterruptedRevisionNode);
}

/** 对话流明说句条目。id 由节点 id 决定：rebuild 与 live 补条目走同一 id，不叠双。 */
export function interruptedRevisionEntry(node: TimelineNode): ChatEntry {
  return {
    id: `${node.node_id}:revision-interrupted`,
    type: "stage_change",
    role: "system",
    content: REVISION_INTERRUPTED_SENTENCE,
    timestamp: node.completed_at ?? node.started_at,
    node_id: node.node_id,
    metadata: {
      title: "运行已中止",
      node_type: node.node_type,
      status: "aborted",
    },
  };
}

type InflightState = Pick<
  WorkspaceWsState,
  | "activeRunId"
  | "activeNodeId"
  | "stage"
  | "flowKind"
  | "sessionStatus"
  | "humanGateClosure"
  | "humanGateTurn"
  | "candidateCompileFailure"
  | "timelineNodes"
>;

/**
 * 当前门是否有门内 provider run 在途，及其形态。从服务端权威事实派生：
 * `active_run_id` 非空 + 活动节点 `status=active`，`author_run` ⇒ 修订中，
 * `reviewer_run` ⇒ 复评中；其余一律 null（不猜）。
 *
 * 让位（返回 null）：已收口 / 终态会话；存在 open|busy|awaiting_confirm 的 live
 * turn（既有 liveTurnStatus 路径接管，行为不变）；候选编译自动重写 running（有自己
 * 的文案与锁）。节点经 `timeline_node_updated` 进终态后自动失效，故陈旧的
 * `activeRunId` 不会永久锁住门卡。
 */
export function selectGateInflightPhase(state: InflightState): GateInflightPhase | null {
  if (
    state.activeRunId === null ||
    state.stage !== "human_confirm" ||
    state.flowKind !== "single_candidate"
  ) {
    return null;
  }
  if (state.humanGateClosure?.decision || terminalGateClosure(state) !== null) {
    return null;
  }
  const turnStatus = state.humanGateTurn?.status;
  if (turnStatus === "open" || turnStatus === "busy" || turnStatus === "awaiting_confirm") {
    return null;
  }
  if (state.candidateCompileFailure?.auto_rewrite?.phase === "running") {
    return null;
  }
  const active =
    state.activeNodeId !== null
      ? state.timelineNodes.find((node) => node.node_id === state.activeNodeId)
      : [...state.timelineNodes].reverse().find((node) => node.status === "active");
  if (!active || active.status !== "active") {
    return null;
  }
  if (active.node_type === "author_run") {
    return "revising";
  }
  if (active.node_type === "reviewer_run") {
    return "reviewing";
  }
  return null;
}
