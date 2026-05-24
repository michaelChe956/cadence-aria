import { describe, expect, it } from "vitest";
import type {
  CodingAttempt,
  CodingAttemptSnapshotResponse,
  CodingWsInMessage,
  CodingWsOutMessage,
  IssueLifecycleResponse,
  NodeDetail,
  TimelineNodeType,
  WsInMessage,
  WsOutMessage,
} from "./types";

describe("workspace websocket protocol types", () => {
  it("accepts protocol v2 inbound messages", () => {
    const note: WsInMessage = { type: "context_note", content: "补充上下文" };
    const start: WsInMessage = {
      type: "start_generation",
      provider_config: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
      reviewer_enabled: true,
    };
    const human: WsInMessage = {
      type: "human_confirm",
      decision: "request-change",
      payload: { description: "补充验收标准" },
    };

    expect(note.type).toBe("context_note");
    expect(start.type).toBe("start_generation");
    expect(human.decision).toBe("request-change");
  });

  it("accepts protocol v2 outbound messages", () => {
    const error: WsOutMessage = {
      type: "protocol_error",
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "context_note not allowed in running",
      context: { stage: "running" },
    };
    const locked: WsOutMessage = {
      type: "provider_locked",
      snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
      locked_at: "2026-05-20T00:00:00Z",
    };

    expect(error.code).toBe("INVALID_MESSAGE_FOR_STAGE");
    expect(locked.snapshot.author).toBe("claude_code");
  });

  it("describes node details used by session snapshots", () => {
    const nodeType: TimelineNodeType = "author_run";
    const detail: NodeDetail = {
      node_id: "timeline_node_001",
      session_id: "workspace_session_0001",
      node_type: nodeType,
      status: "completed",
      agent_role: "author",
      provider: { name: "claude_code", model: "claude_code" },
      messages: [],
      streaming_content: "# Story",
      execution_events: [],
      permission_events: [],
      verdict: null,
      artifact_ref: { artifact_id: "artifact_version_001", version: 1 },
      is_revision: false,
      base_artifact_ref: null,
      started_at: "2026-05-20T00:00:00Z",
      ended_at: "2026-05-20T00:01:00Z",
    };

    expect(detail.node_type).toBe("author_run");
    expect(detail.artifact_ref?.version).toBe(1);
  });

  it("describes coding attempts returned by lifecycle responses", () => {
    const attempt: CodingAttempt = {
      attempt_id: "coding_attempt_0001",
      work_item_id: "work_item_0001",
      attempt_no: 1,
      status: "created",
      stage: "prepare_context",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "master",
      worktree_path: null,
      rework_count: 0,
      head_commit: null,
      push_status: null,
      review_request_url: null,
      created_at: "2026-05-23T00:00:00Z",
      updated_at: "2026-05-23T00:00:00Z",
    };
    const lifecycle = {
      issue: {} as IssueLifecycleResponse["issue"],
      story_specs: [],
      design_specs: [],
      work_items: [
        {
          work_item_id: "work_item_0001",
          issue_id: "issue_0001",
          repository_id: "repository_0001",
          story_spec_ids: [],
          design_spec_ids: [],
          title: "实现爬楼梯",
          plan_status: "confirmed",
          execution_status: "pending",
          latest_attempt: attempt,
        },
      ],
      workspace_sessions: [],
      coding_attempts: [attempt],
    } satisfies IssueLifecycleResponse;

    expect(lifecycle.work_items[0].latest_attempt?.attempt_id).toBe("coding_attempt_0001");
    expect(lifecycle.coding_attempts[0].stage).toBe("prepare_context");
  });

  it("describes coding attempt snapshots and websocket messages", () => {
    const attempt: CodingAttempt = {
      attempt_id: "coding_attempt_0001",
      work_item_id: "work_item_0001",
      attempt_no: 1,
      status: "running",
      stage: "worktree_prepare",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "master",
      worktree_path: "/tmp/repo/.worktrees/aria-work-items/work_item_0001/attempt-1",
      rework_count: 0,
      head_commit: null,
      push_status: null,
      review_request_url: null,
      created_at: "2026-05-23T00:00:00Z",
      updated_at: "2026-05-23T00:00:01Z",
    };
    const snapshot: CodingAttemptSnapshotResponse = {
      attempt,
      provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
      timeline_nodes: [
        {
          id: "coding_node_0001",
          attempt_id: "coding_attempt_0001",
          stage: "worktree_prepare",
          title: "准备 worktree",
          status: "running",
          agent_role: "git",
          summary: null,
          started_at: "2026-05-23T00:00:01Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      active_node_id: "coding_node_0001",
      testing_report: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
    };
    const outbound: CodingWsOutMessage = {
      type: "coding_session_state",
      attempt_id: "coding_attempt_0001",
      status: "running",
      stage: "worktree_prepare",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "master",
      worktree_path: null,
      rework_count: 0,
      max_auto_rework: 2,
      head_commit: null,
      pushed_remote: null,
      provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
      timeline_nodes: snapshot.timeline_nodes,
      active_node_id: "coding_node_0001",
      testing_report: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
    };
    const inbound: CodingWsInMessage = { type: "start_coding" };

    expect(snapshot.active_node_id).toBe("coding_node_0001");
    expect(outbound.type).toBe("coding_session_state");
    expect(inbound.type).toBe("start_coding");
  });
});
