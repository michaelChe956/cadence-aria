import type { LcFreshOperationDto, LcResumeRecoveryDto } from "../api/types";
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

// C1（Task 6.1 / anchor_t19）：LC 恢复 wire 投影夹具——字段值与后端
// anchor_t03（workspace_ws_types tests：durable record → LcResumeRecoveryDto）
// 逐字段一致，供 driver store / observer store / cockpit inbox 三路径的
// 同一恢复事实重建测试共用；native/digest 秘密不在对外 DTO 中。
export function lcFreshOperationFixture(
  overrides: Partial<LcFreshOperationDto> = {},
): LcFreshOperationDto {
  return {
    operation_id: "op-1",
    state: "failed",
    reason: null,
    fresh_node: null,
    ...overrides,
  };
}

export function lcResumeRecoveryFixture(
  overrides: Partial<LcResumeRecoveryDto> = {},
): LcResumeRecoveryDto {
  return {
    recovery_id: "rec-1",
    state: "waiting",
    reason_code: "resume_fingerprint_mismatch",
    provider: "codex",
    failed_node_id: "node-1",
    run_kind: "legacy_draft",
    detected_at: "2026-10-11T00:02:00Z",
    updated_at: "2026-10-11T00:04:00Z",
    // A/B 发布纪律：B 波接齐前 allowed_actions 恒空（前端镜像事实，不推导）。
    allowed_actions: [],
    last_operation: lcFreshOperationFixture(),
    ...overrides,
  };
}
