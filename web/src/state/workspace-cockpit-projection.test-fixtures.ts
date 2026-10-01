import type { ChatEntry } from "./chat-entries";
import type { TimelineNode } from "./workspace-ws-store";

// 从 workspace-cockpit-projection.test.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 投影测试共用的 timeline 节点 / review verdict 夹具工厂。
export function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node_1",
    node_type: "reviewer_run",
    agent: "codex",
    stage: "cross_review",
    round: 1,
    status: "active",
    title: "Review Round 1",
    summary: null,
    started_at: "2026-09-13T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
    ...overrides,
  };
}

export function reviewVerdictEntry(metadata: Record<string, unknown>): ChatEntry {
  return {
    id: "review_verdict:node_1",
    type: "review_verdict",
    role: "reviewer",
    content: "review",
    timestamp: "2026-09-13T00:00:00Z",
    node_id: "node_1",
    metadata,
  };
}
