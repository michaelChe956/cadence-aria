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

describe("useWorkspaceWs websocket messages", () => {
  installWorkspaceWsTestHooks();

  it("stores permission requests and provider status from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "permission_request",
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo test",
        risk_level: "medium",
      });
      harness.ws.receive({
        type: "provider_status",
        status: "waiting_approval",
      });
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toEqual([
      {
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo test",
        risk_level: "medium",
      },
    ]);
    expect(useWorkspaceStore.getState().providerStatus).toBe("waiting_approval");
  });

  it("stores choice requests from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "choice_request",
        id: "choice_001",
        prompt: "请选择下一步",
        options: [
          { id: "continue", label: "继续" },
          { id: "stop", label: "停止" },
        ],
        allow_multiple: false,
        allow_free_text: true,
        source: "ask_user_question",
      });
    });

    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      id: "choice_request:choice_001",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      metadata: {
        request_id: "choice_001",
        allow_multiple: false,
        allow_free_text: true,
        source: "ask_user_question",
      },
    });
  });

  it("stores stage change entries with readable labels and original stage metadata", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "stage_change",
        stage: "author_confirm",
      });
    });

    expect(useWorkspaceStore.getState().stage).toBe("author_confirm");
    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "stage_change",
      role: "system",
      content: "等待作者确认",
      metadata: { stage: "author_confirm" },
    });
    expect(useWorkspaceStore.getState().chatEntries.at(-1)?.content).not.toContain(
      "author_confirm",
    );
  });

  it("stores execution events from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "command_cmd_001",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command: "pwd",
          cwd: "/tmp/repo",
          output: "/tmp/repo\n",
          exit_code: 0,
        },
      });
    });

    expect(useWorkspaceStore.getState().executionEvents).toEqual([
      {
        event_id: "command_cmd_001",
        kind: "command",
        status: "completed",
        title: "Command completed",
        detail: "exit code 0",
        command: "pwd",
        cwd: "/tmp/repo",
        output: "/tmp/repo\n",
        exit_code: 0,
      },
    ]);
  });

  it("uses the concrete command as execution event chat title", () => {
    const harness = renderWorkspaceHook();
    const command =
      "/usr/bin/zsh -lc \"sed -n '1,220p' /home/michael/.codex/superpowers/skills/using-superpowers/SKILL.md\"";

    act(() => {
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "command_cmd_001",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command,
          cwd: "/tmp/repo",
          output: "ok\n",
          exit_code: 0,
        },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "execution_event",
      content: command,
    });
  });

  it("labels work item plan author execution events as author messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_outline",
          node_type: "work_item_plan_outline_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "WorkItemPlan Outline 生成",
          summary: null,
          started_at: "2026-06-23T10:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "provider",
          node_id: "timeline_node_outline",
          agent: "claude_code",
          kind: "provider",
          status: "started",
          title: "Claude Code provider started",
          detail: null,
          command: null,
          cwd: "/tmp/repo",
          output: null,
          exit_code: null,
        },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "execution_event",
      role: "author",
      metadata: expect.objectContaining({ provider: "claude_code" }),
    });
  });

  it("keeps repeated provider lifecycle events scoped to their timeline node", () => {
    const harness = renderWorkspaceHook();

    function workItemPlanOutlineNode(nodeId: string, title: string) {
      return {
        node_id: nodeId,
        node_type: "work_item_plan_outline_run",
        agent: "claude_code",
        stage: "running",
        round: null,
        status: "active",
        title,
        summary: null,
        started_at: "2026-06-23T10:00:00Z",
        completed_at: null,
        duration_ms: null,
        artifact_ref: null,
        provider_config_snapshot: {
          author: "claude_code",
          reviewer: "codex",
          review_rounds: 1,
        },
      };
    }

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: workItemPlanOutlineNode("timeline_node_outline_1", "WorkItemPlan Outline 生成"),
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "provider",
          node_id: "timeline_node_outline_1",
          agent: "claude_code",
          kind: "provider",
          status: "started",
          title: "Claude Code provider started",
          detail: null,
          command: null,
          cwd: "/repo",
          output: null,
          exit_code: null,
        },
      });
      harness.ws.receive({
        type: "timeline_node_created",
        node: workItemPlanOutlineNode(
          "timeline_node_outline_revision",
          "WorkItemPlan Outline 返修",
        ),
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "provider",
          node_id: "timeline_node_outline_revision",
          agent: "claude_code",
          kind: "provider",
          status: "started",
          title: "Claude Code provider started",
          detail: null,
          command: null,
          cwd: "/repo",
          output: null,
          exit_code: null,
        },
      });
    });

    const providerEntries = useWorkspaceStore
      .getState()
      .chatEntries.filter(
        (entry) => entry.type === "execution_event" && entry.metadata?.event_id === "provider",
      );
    expect(providerEntries).toHaveLength(2);
    expect(providerEntries.map((entry) => entry.id)).toEqual([
      "timeline_node_outline_1:execution-provider",
      "timeline_node_outline_revision:execution-provider",
    ]);
    expect(providerEntries.map((entry) => entry.node_id)).toEqual([
      "timeline_node_outline_1",
      "timeline_node_outline_revision",
    ]);
  });

  it("restores sanitized work item plan execution events as prompt and command rows", () => {
    const harness = renderWorkspaceHook("session_work_item_plan_restore_events");
    const promptSize = 42 * 1024;
    const command =
      "/usr/bin/zsh -lc \"sed -n '1,220p' /home/michael/.codex/superpowers/skills/writing-plans/SKILL.md\"";

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_work_item_plan_restore_events",
        workspace_type: "work_item_plan",
        stage: "human_confirm",
        superpowers_enabled: true,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_review",
            node_type: "work_item_plan_outline_review",
            agent: "codex",
            stage: "cross_review",
            round: 1,
            status: "completed",
            title: "WorkItemPlan Outline Review Round 1",
            summary: "需要返修",
            started_at: "2026-06-23T10:00:00Z",
            completed_at: "2026-06-23T10:05:00Z",
            duration_ms: null,
            artifact_ref: "artifact_current",
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_review",
        artifact_versions: [],
        timeline_node_details: {
          timeline_node_review: {
            node_id: "timeline_node_review",
            session_id: "session_work_item_plan_restore_events",
            node_type: "work_item_plan_outline_review",
            status: "completed",
            agent_role: "reviewer",
            provider: { name: "codex", model: "codex" },
            prompt: null,
            messages: [],
            streaming_content: "",
            execution_events: [
              {
                event_id: "prompt",
                node_id: "timeline_node_review",
                agent: "codex",
                kind: "output",
                status: "started",
                title: "Provider Prompt",
                detail: "发送给 Workspace provider 的完整提示词",
                command: null,
                cwd: null,
                output: null,
                exit_code: null,
              },
              {
                event_id: "provider",
                node_id: "timeline_node_review",
                agent: "codex",
                kind: "provider",
                status: "started",
                title: "Codex provider started",
                detail: null,
                command: null,
                cwd: "/repo",
                output: null,
                exit_code: null,
              },
              {
                event_id: "turn",
                node_id: "timeline_node_review",
                agent: "codex",
                kind: "turn",
                status: "completed",
                title: "Turn completed",
                detail: null,
                command: null,
                cwd: "/repo",
                output: null,
                exit_code: null,
              },
              {
                event_id: "call_read_skill",
                node_id: "timeline_node_review",
                agent: "codex",
                kind: "command",
                status: "completed",
                title: "Command completed",
                detail: "exit code 0",
                command,
                cwd: "/repo",
                output: null,
                exit_code: 0,
              },
            ],
            permission_events: [],
            verdict: null,
            artifact_ref: null,
            is_revision: false,
            base_artifact_ref: null,
            started_at: "2026-06-23T10:00:00Z",
            ended_at: "2026-06-23T10:05:00Z",
          },
        },
        timeline_node_summaries: {
          timeline_node_review: {
            node_id: "timeline_node_review",
            node_type: "work_item_plan_outline_review",
            status: "completed",
            agent_role: "reviewer",
            provider_name: "codex",
            prompt_size: promptSize,
            prompt_preview: "prompt preview",
            stream_size: 0,
            stream_preview: null,
            execution_event_count: 4,
            has_large_outputs: true,
            artifact_ref: "artifact_current/v1",
            started_at: "2026-06-23T10:00:00Z",
            ended_at: "2026-06-23T10:05:00Z",
          },
        },
        active_run_id: null,
      });
    });

    const state = useWorkspaceStore.getState();
    expect(
      state.chatEntries.filter(
        (entry) => entry.type === "execution_event" && entry.content.includes("Provider Prompt"),
      ),
    ).toEqual([
      expect.objectContaining({
        id: "timeline_node_review:provider-prompt",
        role: "reviewer",
        content: "WorkItemPlan Outline Review Round 1 · Provider Prompt · 约 42KB",
        content_ref: { kind: "provider_prompt", nodeId: "timeline_node_review" },
        content_size: promptSize,
        has_full_content: true,
      }),
    ]);
    expect(state.chatEntries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "timeline_node_review:execution-provider",
          content: "Codex provider started",
          metadata: expect.objectContaining({ provider: "codex" }),
        }),
        expect.objectContaining({
          id: "timeline_node_review:execution-turn",
          content: "Turn completed",
        }),
        expect.objectContaining({
          id: "timeline_node_review:execution-call_read_skill",
          content: command,
          content_ref: {
            kind: "execution_output",
            nodeId: "timeline_node_review",
            eventId: "call_read_skill",
          },
          has_full_content: false,
        }),
      ]),
    );
  });
});
