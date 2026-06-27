import { act } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../state/chat-entries";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  MockWebSocket,
  installWorkspaceWsTestHooks,
  makeBatchArtifactPayload,
  makeCompileArtifactPayload,
  makeDraftArtifactPayload,
  makeOutlineArtifactPayload,
  makeWorkItemPlanCandidate,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";

describe("useWorkspaceWs timeline state", () => {
  installWorkspaceWsTestHooks();

  it("stores timeline websocket messages by node", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_001",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "active",
          title: "Review Round 1",
          summary: null,
          started_at: "2026-05-19T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 2,
          },
        },
      });
      harness.ws.receive({
        type: "stage_change",
        stage: "cross_review",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "reviewer",
        content: "review output",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "review_complete",
        node_id: "timeline_node_001",
        round: 1,
        verdict: "pass",
        comments: "审核通过",
        summary: "可以确认",
        findings: [
          {
            severity: "optional",
            message: "建议补充说明",
            evidence: "当前版本可用",
            impact: "不影响下一阶段",
            required_action: "可后续优化",
          },
        ],
        review_gate: "user_triage_required",
      });
    });

    const state = useWorkspaceStore.getState();
    expect(state.selectedNodeId).toBe("timeline_node_001");
    expect(state.nodeDetails.timeline_node_001.streaming_content).toBe("review output");
    expect(state.nodeDetails.timeline_node_001.verdict).toMatchObject({
      summary: "可以确认",
      review_gate: "user_triage_required",
      findings: [expect.objectContaining({ message: "建议补充说明" })],
    });
    expect(
      state.chatEntries.find((entry) => entry.type === "review_verdict")?.metadata,
    ).toMatchObject({
      review_gate: "user_triage_required",
      findings: [expect.objectContaining({ required_action: "可后续优化" })],
    });
  });

  it("maps websocket events into chat entries", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_chat",
        workspace_type: "story",
        stage: "prepare_context",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_000",
            node_type: "context_note",
            agent: null,
            stage: "prepare_context",
            round: null,
            status: "completed",
            title: "补充上下文",
            summary: null,
            started_at: "2026-05-21T09:59:00Z",
            completed_at: "2026-05-21T09:59:05Z",
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
          {
            node_id: "timeline_node_001",
            node_type: "author_run",
            agent: "claude_code",
            stage: "running",
            round: null,
            status: "active",
            title: "Story Spec 生成",
            summary: null,
            started_at: "2026-05-21T10:00:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_001",
        artifact_versions: [],
        timeline_node_details: {
          timeline_node_000: {
            node_id: "timeline_node_000",
            session_id: "session_chat",
            node_type: "context_note",
            status: "completed",
            agent_role: null,
            provider: null,
            prompt: null,
            messages: [],
            streaming_content: "需要支持手机号登录",
            execution_events: [],
            permission_events: [],
            verdict: null,
            artifact_ref: null,
            is_revision: false,
            base_artifact_ref: null,
            started_at: "2026-05-21T09:59:00Z",
            ended_at: "2026-05-21T09:59:05Z",
          },
          timeline_node_001: {
            node_id: "timeline_node_001",
            session_id: "session_chat",
            node_type: "author_run",
            status: "active",
            agent_role: "author",
            provider: { name: "claude_code", model: "claude-opus-4" },
            prompt: null,
            messages: [],
            streaming_content: "",
            execution_events: [],
            permission_events: [],
            verdict: null,
            artifact_ref: null,
            is_revision: false,
            base_artifact_ref: null,
            started_at: "2026-05-21T10:00:00Z",
            ended_at: null,
          },
        },
        active_run_id: "run-001",
      });
      harness.ws.receive({
        type: "provider_locked",
        snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        locked_at: "2026-05-21T10:00:00Z",
      });
      harness.ws.receive({ type: "stage_change", stage: "running" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "第一段",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "第二段",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "message_complete",
        message_id: "msg_001",
        checkpoint_id: "checkpoint_001",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "exec-1",
          node_id: "timeline_node_001",
          agent: "claude_code",
          kind: "command",
          status: "completed",
          title: "读取认证模块",
          detail: "exit code 0",
          command: "sed -n '1,120p' src/auth.rs",
          cwd: "/repo",
          output: null,
          exit_code: 0,
        },
      });
      harness.ws.receive({
        type: "permission_request",
        id: "permission-1",
        tool_name: "shell",
        description: "cargo test",
        risk_level: "medium",
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        markdown: "# Story",
      });
      harness.ws.receive({
        type: "review_complete",
        node_id: "timeline_node_001",
        round: 1,
        verdict: "pass",
        comments: "审核通过",
        summary: "可以确认",
        findings: [
          {
            severity: "minor",
            message: "建议优化标题",
            evidence: "标题可读但不够具体",
            impact: "不影响下一阶段",
            required_action: "可后续调整",
          },
        ],
        review_gate: "user_confirm_allowed",
      });
      harness.ws.receive({ type: "error", message: "阶段不允许" });
    });

    const state = useWorkspaceStore.getState();
    expect(state.chatEntries.map((entry) => entry.type)).toEqual([
      "context_note",
      "start_generation",
      "stage_change",
      "provider_stream",
      "execution_event",
      "permission_request",
      "artifact_update",
      "review_verdict",
      "error",
    ]);
    expect(state.chatEntries[0]).toMatchObject({
      role: "user",
      content: "需要支持手机号登录",
      node_id: "timeline_node_000",
    });
    expect(state.chatEntries[3]).toMatchObject({
      role: "author",
      content: "第一段第二段",
      node_id: "timeline_node_001",
    });
    expect(state.chatEntries[5].metadata).toMatchObject({
      request_id: "permission-1",
      risk_level: "medium",
    });
    expect(state.chatEntries[7]).toMatchObject({
      role: "reviewer",
      content: "可以确认",
      metadata: expect.objectContaining({
        review_gate: "user_confirm_allowed",
        findings: [expect.objectContaining({ message: "建议优化标题" })],
      }),
    });
    expect(state.activeStreamEntryId).toBeNull();
  });

  it("adds a gate prompt entry when the stage changes to human_confirm", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_chat",
        workspace_type: "story",
        stage: "running",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_human_confirm",
            node_type: "human_confirm",
            agent: null,
            stage: "human_confirm",
            round: null,
            status: "active",
            title: "人工确认",
            summary: "等待人工确认",
            started_at: "2026-05-21T10:03:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_human_confirm",
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
      });
      harness.ws.receive({ type: "stage_change", stage: "human_confirm" });
    });

    const state = useWorkspaceStore.getState();
    expect(state.chatEntries.at(-1)).toMatchObject({
      type: "gate_prompt",
      role: "system",
      node_id: "timeline_node_human_confirm",
    });
  });

  it("ignores late provider stream chunks after an abort returns to prepare_context", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_abort",
        workspace_type: "story",
        stage: "running",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_aborted",
            node_type: "author_run",
            agent: "claude_code",
            stage: "running",
            round: null,
            status: "active",
            title: "Story Spec 生成",
            summary: null,
            started_at: "2026-06-19T10:00:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_aborted",
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: "run-1",
      });
      harness.ws.receive({ type: "stage_change", stage: "prepare_context" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "late output",
        node_id: "timeline_node_aborted",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.type === "provider_stream"),
    ).toBe(false);
    expect(useWorkspaceStore.getState().streamBuffers).toEqual({});
    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("stores protocol errors and provider lock events from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "protocol_error",
        code: "INVALID_MESSAGE_FOR_STAGE",
        message: "阶段不允许",
      });
      harness.ws.receive({
        type: "provider_locked",
        snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        locked_at: "2026-05-20T14:35:00Z",
      });
      harness.ws.receive({ type: "pong" });
    });

    const state = useWorkspaceStore.getState();
    expect(state.protocolError).toEqual({
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "阶段不允许",
    });
    expect(state.providerLocked).toBe(true);
    expect(state.providerSnapshot).toEqual({
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    });
  });

  it("marks stale choice requests rejected when the server reports an unmatched choice id", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().appendChatEntry({
      id: "choice_request:choice_001",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [{ id: "continue", label: "继续" }],
      },
    } as ChatEntry);
    useWorkspaceStore.getState().resolveChoiceRequest("choice_001", ["continue"], null);

    act(() => {
      harness.ws.receive({
        type: "protocol_error",
        code: "CHOICE_ID_UNMATCHED",
        message: "ChoiceResponse id=choice_001 not found in pending",
        context: { choice_id: "choice_001" },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "choice_request:choice_001",
        resolved: true,
        metadata: expect.objectContaining({
          rejected: true,
          rejection_reason: "ChoiceResponse id=choice_001 not found in pending",
        }),
      }),
      expect.objectContaining({
        type: "error",
        content: "CHOICE_ID_UNMATCHED · ChoiceResponse id=choice_001 not found in pending",
      }),
    ]);
  });

  it("sends review decision responses when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendReviewDecision("continue_with_context", " 补充边界条件 ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "review_decision_response",
        decision: "continue_with_context",
        extra_context: "补充边界条件",
      }),
    ]);
  });
});
