import { describe, expect, it } from "vitest";
import type { ChatEntry } from "./chat-entries";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "./workspace-content-cache";
import { selectPrepareContextNotes, useWorkspaceStore } from "./workspace-ws-store";
import {
  installWorkspaceStoreTestHooks,
  makeCompileArtifactPayload,
  makeContextBlockerArtifactPayload,
  makeDraftArtifactPayload,
  makeNodeDetail,
  makeOutlineArtifactPayload,
  makeWorkItemPlanCandidate,
} from "./workspace-ws-store.test-utils";

describe("workspace ws store content entries", () => {
  installWorkspaceStoreTestHooks();

  it("rebuilds only the latest provider prompt event for a node", () => {
    const store = useWorkspaceStore.getState();
    const firstPrompt = "delta prompt";
    const secondPrompt = "full prompt ".repeat(200);

    store.setSessionState({
      session_id: "session_revision_prompt_replace",
      workspace_type: "design",
      stage: "revision",
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
          status: "failed",
          title: "返修 Round 1",
          summary: "运行已中止",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
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
        timeline_node_revision: makeNodeDetail({
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent_role: "author",
          provider: { name: "codex", model: "codex" },
          prompt: secondPrompt,
          execution_events: [
            {
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
            {
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
          ],
        }),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    const promptEntries = useWorkspaceStore
      .getState()
      .chatEntries.filter(
        (entry) => entry.type === "execution_event" && entry.content.includes("Provider Prompt"),
      );
    expect(promptEntries).toHaveLength(1);
    expect(promptEntries[0]).toMatchObject({
      id: "timeline_node_revision:provider-prompt",
      content_size: secondPrompt.length,
      metadata: expect.objectContaining({ event_id: "revision_prompt_full" }),
    });
  });

  it("does not duplicate artifact markdown in rebuilt chat entry metadata", () => {
    const store = useWorkspaceStore.getState();
    const hugeMarkdown = "# Artifact\n" + "content\n".repeat(10_000);

    store.setSessionState({
      session_id: "session_huge_artifact",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: hugeMarkdown,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "completed",
          title: "Story 生成",
          summary: "生成完成",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [
        {
          version: 1,
          markdown: hugeMarkdown,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-05-26T10:01:00Z",
          source_node_id: "timeline_node_001",
        },
      ],
      timeline_node_details: {
        timeline_node_001: makeNodeDetail(),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    const artifactEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "artifact_update");
    expect(artifactEntry?.metadata?.markdown).toBeUndefined();
    expect(JSON.stringify(artifactEntry)).not.toContain(hugeMarkdown.slice(0, 100));
  });

  it("does not duplicate provider prompt output in rebuilt chat entry metadata", () => {
    const store = useWorkspaceStore.getState();
    const hugePrompt = "[system]\n" + "prompt line\n".repeat(10_000);

    store.setSessionState({
      session_id: "session_huge_prompt",
      workspace_type: "story",
      stage: "running",
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
          title: "Story 生成",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
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
        timeline_node_001: makeNodeDetail({
          prompt: hugePrompt,
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    const promptEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.id === "timeline_node_001:provider-prompt");
    expect(promptEntry?.metadata?.output).toBeUndefined();
    expect(JSON.stringify(promptEntry)).not.toContain(hugePrompt.slice(0, 100));
  });

  it("does not duplicate provider prompt execution event output in rebuilt chat entry metadata", () => {
    const store = useWorkspaceStore.getState();
    const hugePrompt = "[system]\n" + "prompt line\n".repeat(10_000);

    store.setSessionState({
      session_id: "session_huge_prompt_event",
      workspace_type: "story",
      stage: "running",
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
          title: "Story 生成",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
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
        timeline_node_001: makeNodeDetail({
          execution_events: [
            {
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
          ],
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

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

  it("rebuilds lightweight provider content entries from node summaries", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_summary_only",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_034",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          round: 1,
          status: "completed",
          title: "Large Provider Stream 33",
          summary: "large provider summary",
          started_at: "2026-06-06T00:00:00Z",
          completed_at: "2026-06-06T00:00:01Z",
          duration_ms: 1000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 5,
          },
        },
      ],
      active_node_id: "timeline_node_034",
      artifact_versions: [],
      timeline_node_details: {},
      timeline_node_summaries: {
        timeline_node_034: {
          node_id: "timeline_node_034",
          node_type: "author_run",
          status: "completed",
          agent_role: "author",
          provider_name: "codex",
          prompt_size: 120_000,
          prompt_preview: "完整提示词 large-prompt-0 preview",
          stream_size: 80,
          stream_preview: "stream summary",
          execution_event_count: 1,
          has_large_outputs: true,
          artifact_ref: null,
          started_at: "2026-06-06T00:00:00Z",
          ended_at: "2026-06-06T00:00:01Z",
        },
      },
      active_run_id: "run-1",
    });

    const entries = useWorkspaceStore.getState().chatEntries;

    expect(entries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          content: expect.stringContaining("Provider Prompt"),
          content_ref: { kind: "provider_prompt", nodeId: "timeline_node_034" },
          content_size: 120_000,
        }),
        expect.objectContaining({
          content: expect.stringContaining("Execution Output"),
          content_ref: {
            kind: "execution_output",
            nodeId: "timeline_node_034",
            eventId: "timeline_node_034_output",
          },
        }),
      ]),
    );
  });

  it("preserves active stream chunk order while avoiding duplicate historical entries", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "workspace_session_stream",
      workspace_type: "story",
      stage: "running",
      superpowers_enabled: true,
      openspec_enabled: true,
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          status: "active",
          title: "Story Spec 生成",
          started_at: "2026-06-06T00:00:00Z",
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: "run-1",
    });

    useWorkspaceStore
      .getState()
      .appendBufferedStreamChunk("A", "timeline_node_001", "author");
    useWorkspaceStore
      .getState()
      .appendBufferedStreamChunk("B", "timeline_node_001", "author");
    useWorkspaceStore.getState().flushBufferedStream("timeline_node_001");

    const streamEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.id === "timeline_node_001:stream-active");

    expect(streamEntry?.content).toBe("AB");
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.filter((entry) => entry.id === "timeline_node_001:stream-active"),
    ).toHaveLength(1);
  });

  it("clears buffered stream state after completing a node stream", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "workspace_session_stream_complete",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          status: "active",
          title: "Story Spec 生成",
          started_at: "2026-06-06T00:00:00Z",
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: "run-1",
    });

    store.appendStreamChunk("A", "timeline_node_001");
    store.appendStreamChunk("B", "timeline_node_001");
    store.appendBufferedStreamChunk("A", "timeline_node_001", "author");
    store.appendBufferedStreamChunk("B", "timeline_node_001", "author");

    store.completeBufferedStream("timeline_node_001", "msg_001", "checkpoint_001");

    const state = useWorkspaceStore.getState();
    const streamEntry = state.chatEntries.find(
      (entry) => entry.id === "timeline_node_001:stream-active",
    );
    expect(streamEntry?.content).toBe("AB");
    expect(state.nodeDetails.timeline_node_001.messages.at(-1)?.content).toBe("AB");
    expect(state.streamBuffers.timeline_node_001).toBeUndefined();
  });

  it("stores review verdict and pending decision by node", () => {
    const store = useWorkspaceStore.getState();
    store.setNodeVerdict("timeline_node_003", {
      verdict: "revise",
      comments: "需要补充失败路径",
      summary: "补充失败路径",
    });
    store.setPendingDecision({
      node_id: "timeline_node_004",
      round: 1,
      options: ["continue", "continue_with_context", "human_intervene"],
    });

    expect(useWorkspaceStore.getState().nodeDetails.timeline_node_003.verdict).toEqual({
      verdict: "revise",
      comments: "需要补充失败路径",
      summary: "补充失败路径",
    });
    expect(useWorkspaceStore.getState().pendingDecision?.node_id).toBe("timeline_node_004");
  });

  it("stores and clears protocol errors", () => {
    const store = useWorkspaceStore.getState();

    store.setProtocolError({ code: "INVALID_MESSAGE_FOR_STAGE", message: "阶段不允许" });

    expect(useWorkspaceStore.getState().protocolError).toEqual({
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "阶段不允许",
    });

    store.setProtocolError(null);

    expect(useWorkspaceStore.getState().protocolError).toBeNull();
  });

  it("stores and clears provider locked snapshots", () => {
    const store = useWorkspaceStore.getState();

    store.setProviderLocked({
      snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
      locked_at: "2026-05-20T14:35:00Z",
    });

    expect(useWorkspaceStore.getState().providerLocked).toBe(true);
    expect(useWorkspaceStore.getState().providerSnapshot).toEqual({
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    });
    expect(useWorkspaceStore.getState().providerLockedAt).toBe("2026-05-20T14:35:00Z");

    store.setProviderLocked(null);

    expect(useWorkspaceStore.getState().providerLocked).toBe(false);
    expect(useWorkspaceStore.getState().providerSnapshot).toBeNull();
    expect(useWorkspaceStore.getState().providerLockedAt).toBeNull();
  });

  it("records live artifact updates as artifact versions", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      providers: { author: "fake", reviewer: "codex" },
      activeNodeId: "node-author-1",
    });

    store.setArtifact("# Draft v1", 1);
    store.setArtifact("# Draft v2", 2);

    expect(useWorkspaceStore.getState().artifact).toBe("# Draft v2");
    expect(useWorkspaceStore.getState().artifactVersions).toMatchObject([
      {
        version: 1,
        markdown: "# Draft v1",
        generated_by: "fake",
        source_node_id: "node-author-1",
      },
      {
        version: 2,
        markdown: "# Draft v2",
        generated_by: "fake",
        source_node_id: "node-author-1",
      },
    ]);
  });
});
