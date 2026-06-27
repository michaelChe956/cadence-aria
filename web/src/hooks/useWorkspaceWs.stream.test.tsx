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

describe("useWorkspaceWs stream reconstruction", () => {
  installWorkspaceWsTestHooks();

  it("does not duplicate realtime provider prompt execution event output in chat entry metadata", () => {
    const harness = renderWorkspaceHook();
    const hugePrompt = "[system]\n" + "prompt line\n".repeat(10_000);

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_prompt_event",
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
          timeline_node_001: {
            node_id: "timeline_node_001",
            session_id: "session_prompt_event",
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
        type: "execution_event",
        event: {
          event_id: "timeline_node_001_prompt",
          node_id: "timeline_node_001",
          agent: "claude_code",
          kind: "output",
          status: "started",
          title: "Provider Prompt",
          detail: "发送给 Workspace provider 的完整提示词",
          command: null,
          cwd: null,
          output: hugePrompt,
          exit_code: null,
        },
      });
    });

    const promptEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.id === "timeline_node_001:provider-prompt");
    expect(promptEntry?.metadata?.output).toBeUndefined();
    expect(JSON.stringify(promptEntry)).not.toContain(hugePrompt.slice(0, 100));
    expect(promptEntry).toEqual(
      expect.objectContaining({
        content_ref: { kind: "provider_prompt", nodeId: "timeline_node_001" },
        content_size: hugePrompt.length,
        has_full_content: true,
      }),
    );
  });

  it("replaces previous realtime provider prompt entry for the same node", () => {
    const harness = renderWorkspaceHook("session_prompt_replace");
    const firstPrompt = "delta prompt";
    const secondPrompt = "full prompt ".repeat(200);

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_prompt_replace",
        workspace_type: "design",
        stage: "revision",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "codex", reviewer: "claude_code" },
        timeline_nodes: [
          {
            node_id: "timeline_node_revision",
            node_type: "revision",
            agent: "codex",
            stage: "revision",
            round: 1,
            status: "active",
            title: "返修 Round 1",
            summary: null,
            started_at: "2026-05-21T10:00:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "codex",
              reviewer: "claude_code",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_revision",
        artifact_versions: [],
        timeline_node_details: {
          timeline_node_revision: {
            node_id: "timeline_node_revision",
            session_id: "session_prompt_replace",
            node_type: "revision",
            status: "active",
            agent_role: "author",
            provider: { name: "codex", model: "codex" },
            prompt: null,
            messages: [],
            streaming_content: "",
            execution_events: [],
            permission_events: [],
            verdict: null,
            artifact_ref: null,
            is_revision: true,
            base_artifact_ref: null,
            started_at: "2026-05-21T10:00:00Z",
            ended_at: null,
          },
        },
        active_run_id: "run-001",
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "revision_prompt_delta",
          node_id: "timeline_node_revision",
          agent: "codex",
          kind: "output",
          status: "started",
          title: "Provider Prompt",
          detail: "发送给 Workspace provider 的完整提示词",
          command: null,
          cwd: null,
          output: firstPrompt,
          exit_code: null,
        },
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "revision_prompt_full",
          node_id: "timeline_node_revision",
          agent: "codex",
          kind: "output",
          status: "started",
          title: "Provider Prompt",
          detail: "发送给 Workspace provider 的完整提示词",
          command: null,
          cwd: null,
          output: secondPrompt,
          exit_code: null,
        },
      });
    });

    const promptEntries = useWorkspaceStore
      .getState()
      .chatEntries.filter(
        (entry) => entry.type === "execution_event" && entry.content.includes("Provider Prompt"),
      );
    expect(promptEntries).toHaveLength(1);
    expect(promptEntries[0]).toMatchObject({
      id: "timeline_node_revision:provider-prompt",
      content: `返修 Round 1 · Provider Prompt · 约 ${Math.ceil(secondPrompt.length / 1024)}KB`,
      content_size: secondPrompt.length,
      metadata: expect.objectContaining({ event_id: "revision_prompt_full" }),
    });
  });

  it("annotates reviewer stream and tool calls with the reviewer provider", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "active",
          title: "Review Round 1",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
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
        content: "reviewing",
        node_id: "timeline_node_reviewer",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });
    act(() => {
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "command_cmd_001",
          node_id: "timeline_node_reviewer",
          agent: "codex",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command: "git diff --stat",
          cwd: "/tmp/repo",
          output: "ok\n",
          exit_code: 0,
        },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "provider_stream",
          role: "reviewer",
          metadata: expect.objectContaining({ provider: "codex" }),
        }),
        expect.objectContaining({
          type: "execution_event",
          role: "reviewer",
          content: "git diff --stat",
          metadata: expect.objectContaining({ agent: "codex" }),
        }),
      ]),
    );
  });

  it("keeps work item plan stream chunks when active run arrives before provider stage", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_work_item_plan_progress",
        workspace_type: "work_item_plan",
        stage: "prepare_context",
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_work_item_plan_author",
            node_type: "author_run",
            agent: "claude_code",
            stage: "running",
            round: null,
            status: "active",
            title: "Work Item Plan 生成",
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
        active_node_id: "timeline_node_work_item_plan_author",
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: "run-work-item-plan-1",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "Fake Work Item Plan streaming draft",
        node_id: "timeline_node_work_item_plan_author",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "author",
        content: "Fake Work Item Plan streaming draft",
        node_id: "timeline_node_work_item_plan_author",
        metadata: expect.objectContaining({ provider: "claude_code" }),
      }),
    ]);
  });

  it("routes work item plan auto revision stream chunks to the revision node", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_work_item_plan_auto_revision_1",
          node_type: "revision",
          agent: "claude_code",
          stage: "revision",
          round: 1,
          status: "active",
          title: "Work Item Plan 自动返修 Round 1",
          summary: "根据 Work Item Plan 校验结果自动返修",
          started_at: "2026-06-21T10:01:00Z",
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
      harness.ws.receive({ type: "stage_change", stage: "revision" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "Fake Work Item Plan streaming draft",
        node_id: "timeline_node_work_item_plan_auto_revision_1",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    const streamEntry = useWorkspaceStore
      .getState()
      .chatEntries.find(
        (entry) => entry.node_id === "timeline_node_work_item_plan_auto_revision_1",
      );
    expect(streamEntry).toMatchObject({
      type: "provider_stream",
      role: "author",
      node_id: "timeline_node_work_item_plan_auto_revision_1",
      content: "Fake Work Item Plan streaming draft",
    });
  });

  it("stops appending pre-stage stream chunks after prepare_context invalidates the active run", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_work_item_plan_progress",
        workspace_type: "work_item_plan",
        stage: "prepare_context",
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_work_item_plan_author",
            node_type: "author_run",
            agent: "claude_code",
            stage: "running",
            round: null,
            status: "active",
            title: "Work Item Plan 生成",
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
        active_node_id: "timeline_node_work_item_plan_author",
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: "run-work-item-plan-1",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "Fake Work Item Plan streaming draft",
        node_id: "timeline_node_work_item_plan_author",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    act(() => {
      harness.ws.receive({ type: "stage_change", stage: "prepare_context" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "late output",
        node_id: "timeline_node_work_item_plan_author",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    const streamEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "provider_stream");
    expect(streamEntry?.content).toBe("Fake Work Item Plan streaming draft");
    expect(
      useWorkspaceStore.getState().streamBuffers.timeline_node_work_item_plan_author?.chunks,
    ).toEqual([]);
  });

  it("rejects stale chunks for an invalidated node after a new run enters running", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_work_item_plan_progress",
        workspace_type: "work_item_plan",
        stage: "prepare_context",
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_old_author",
            node_type: "author_run",
            agent: "claude_code",
            stage: "running",
            round: null,
            status: "active",
            title: "旧 Work Item Plan 生成",
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
        active_node_id: "timeline_node_old_author",
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: "run-old",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "旧 run 初始输出",
        node_id: "timeline_node_old_author",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    act(() => {
      harness.ws.receive({ type: "stage_change", stage: "prepare_context" });
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_new_author",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "新 Work Item Plan 生成",
          summary: null,
          started_at: "2026-06-19T10:01:00Z",
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
      harness.ws.receive({ type: "stage_change", stage: "running" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "旧 run 迟到输出",
        node_id: "timeline_node_old_author",
      });
    });
    act(() => {
      vi.advanceTimersByTime(80);
    });

    const oldStreamEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.node_id === "timeline_node_old_author");
    expect(oldStreamEntry?.content).toBe("旧 run 初始输出");
    expect(useWorkspaceStore.getState().streamBuffers.timeline_node_old_author?.chunks).toEqual([]);
  });
});
