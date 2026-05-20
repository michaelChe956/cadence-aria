import { describe, expect, it } from "vitest";
import type { NodeDetail, TimelineNodeType, WsInMessage, WsOutMessage } from "./types";

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
});
