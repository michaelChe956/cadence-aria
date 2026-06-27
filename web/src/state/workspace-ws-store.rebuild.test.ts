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

describe("workspace ws store chat rebuild", () => {
  installWorkspaceStoreTestHooks();

  it("groups stream chunks and execution events by timeline node", () => {
    const store = useWorkspaceStore.getState();
    store.addTimelineNode({
      node_id: "timeline_node_002",
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
    });

    store.appendStreamChunk("review output", "timeline_node_002");
    store.upsertExecutionEvent({
      event_id: "turn_001",
      node_id: "timeline_node_002",
      agent: "codex",
      kind: "turn",
      status: "completed",
      title: "Review turn",
      detail: null,
      command: null,
      cwd: null,
      output: null,
      exit_code: null,
    });
    store.completeMessage("msg_002", "checkpoint_002", "timeline_node_002");

    const detail = useWorkspaceStore.getState().nodeDetails.timeline_node_002;
    expect(detail.streaming_content).toBe("");
    expect(detail.messages).toEqual([
      expect.objectContaining({
        id: "msg_002",
        role: "assistant",
        content: "review output",
      }),
    ]);
    expect(detail.execution_events).toHaveLength(1);
    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("rebuilds reviewer command events with command titles and provider metadata", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_reviewer_tools",
      workspace_type: "story",
      stage: "cross_review",
      messages: [],
      checkpoints: [],
      artifact: "# Draft",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
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
      ],
      active_node_id: "timeline_node_reviewer",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_reviewer: makeNodeDetail({
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent_role: "reviewer",
          provider: { name: "codex", model: "gpt-5" },
          streaming_content: "reviewing diff",
          execution_events: [
            {
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
          ],
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "reviewer",
        content: "reviewing diff",
        metadata: expect.objectContaining({ provider: "codex" }),
      }),
      expect.objectContaining({
        type: "execution_event",
        role: "reviewer",
        content: "git diff --stat",
        metadata: expect.objectContaining({ agent: "codex" }),
      }),
    ]);
  });

  it("rebuilds work item plan author stream from author_run timeline node content", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_progress",
      workspace_type: "work_item_plan",
      stage: "running",
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
      timeline_node_details: {
        timeline_node_work_item_plan_author: makeNodeDetail({
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          streaming_content: "Fake Work Item Plan streaming draft",
        }),
      },
      active_run_id: null,
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

  it("rebuilds work item plan outline provider events as author messages", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_outline",
      workspace_type: "work_item_plan",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
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
      ],
      active_node_id: "timeline_node_outline",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_outline: makeNodeDetail({
          node_id: "timeline_node_outline",
          node_type: "work_item_plan_outline_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          execution_events: [
            {
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
          ],
        }),
      },
      active_run_id: null,
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "execution_event",
        role: "author",
        content: "Claude Code provider started",
        node_id: "timeline_node_outline",
        metadata: expect.objectContaining({ provider: "claude_code" }),
      }),
    ]);
  });

  it("does not rebuild work item plan provider stream from start_generation nodes", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_start_generation",
      workspace_type: "work_item_plan",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_start_generation",
          node_type: "start_generation",
          agent: null,
          stage: "prepare_context",
          round: null,
          status: "completed",
          title: "开始生成",
          summary: null,
          started_at: "2026-06-19T10:00:00Z",
          completed_at: "2026-06-19T10:00:00Z",
          duration_ms: 0,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_start_generation",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_start_generation: makeNodeDetail({
          node_id: "timeline_node_start_generation",
          node_type: "start_generation",
          streaming_content: "Fake Work Item Plan streaming draft",
        }),
      },
      active_run_id: null,
    });

    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.type === "provider_stream"),
    ).toBe(false);
    expect(useWorkspaceStore.getState().chatEntries).toContainEqual(
      expect.objectContaining({
        type: "start_generation",
        role: "system",
        node_id: "timeline_node_start_generation",
      }),
    );
  });

  it("restores completed work item plan author stream while focusing active author confirm", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_author_confirm",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
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
          status: "completed",
          title: "Work Item Plan 生成",
          summary: "WorkItemPlan provider 输出完成",
          started_at: "2026-06-21T10:00:00Z",
          completed_at: "2026-06-21T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_version_001",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
        {
          node_id: "timeline_node_author_confirm",
          node_type: "author_confirm",
          agent: null,
          stage: "author_confirm",
          round: null,
          status: "active",
          title: "Author 结果确认",
          summary: "WorkItemPlan 候选已生成，等待确认",
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
      ],
      active_node_id: "timeline_node_author_confirm",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_work_item_plan_author: makeNodeDetail({
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          streaming_content: "Fake Work Item Plan streaming draft",
        }),
        timeline_node_author_confirm: makeNodeDetail({
          node_id: "timeline_node_author_confirm",
          node_type: "author_confirm",
          streaming_content: "",
        }),
      },
      active_run_id: null,
    });

    expect(useWorkspaceStore.getState().activeNodeId).toBe("timeline_node_author_confirm");
    expect(useWorkspaceStore.getState().chatEntries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "provider_stream",
          role: "author",
          node_id: "timeline_node_work_item_plan_author",
          content: "Fake Work Item Plan streaming draft",
        }),
        expect.objectContaining({
          type: "stage_change",
          role: "system",
          node_id: "timeline_node_author_confirm",
        }),
      ]),
    );
  });

  it.each([
    ["story", "已按 reviewer 意见返修 Story Spec"],
    ["design", "已按 reviewer 意见返修 Design Spec"],
    ["work_item", "已按 reviewer 意见返修 Work Item"],
    ["work_item_plan", "正在返修 Work Item Plan"],
  ])("rebuilds revision nodes as author chat entries for %s workspaces", (workspaceType, streamContent) => {
    const store = useWorkspaceStore.getState();
    store.reset();
    store.setSessionState({
      session_id: `session_revision_${workspaceType}`,
      workspace_type: workspaceType,
      stage: "revision",
      messages: [],
      checkpoints: [],
      artifact: "# Draft",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent: "claude_code",
          stage: "revision",
          round: 1,
          status: "completed",
          title: "返修 Round 1",
          summary: "生成完成",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_revision",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_revision: makeNodeDetail({
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent_role: null,
          provider: { name: "claude_code", model: "claude-opus-4" },
          streaming_content: streamContent,
          is_revision: true,
        }),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "author",
        content: streamContent,
        metadata: expect.objectContaining({ provider: "claude_code" }),
      }),
    ]);
  });

  it("deduplicates snapshot execution events before rebuilding chat entries", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_duplicate_events",
      workspace_type: "story",
      stage: "cross_review",
      messages: [],
      checkpoints: [],
      artifact: "# Draft",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "completed",
          title: "Review Round 1",
          summary: "需要返修",
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
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_reviewer: makeNodeDetail({
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent_role: "reviewer",
          provider: { name: "codex", model: "gpt-5" },
          execution_events: [
            {
              event_id: "command_cmd_001",
              node_id: "timeline_node_reviewer",
              agent: "codex",
              kind: "command",
              status: "started",
              title: "Command started",
              detail: null,
              command: "git diff --stat",
              cwd: "/tmp/repo",
              output: null,
              exit_code: null,
            },
            {
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
          ],
        }),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    expect(
      useWorkspaceStore.getState().nodeDetails.timeline_node_reviewer.execution_events,
    ).toEqual([
      expect.objectContaining({
        event_id: "command_cmd_001",
        status: "completed",
        output: "ok\n",
      }),
    ]);
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.filter(
          (entry) => entry.id === "timeline_node_reviewer:execution-command_cmd_001",
        ),
    ).toHaveLength(1);
  });

  it.each(["story", "design", "work_item"])(
    "rebuilds system timeline nodes as chat anchors for %s workspaces",
    (workspaceType) => {
      const store = useWorkspaceStore.getState();
      store.reset();
      store.setSessionState({
        session_id: `session_review_decision_anchor_${workspaceType}`,
        workspace_type: workspaceType,
        stage: "review_decision",
        messages: [],
        checkpoints: [],
        artifact: "# Draft",
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_decision",
            node_type: "review_decision",
            agent: null,
            stage: "review_decision",
            round: 1,
            status: "completed",
            title: "Review Decision Round 1",
            summary: "已选择返修",
            started_at: "2026-05-26T10:02:00Z",
            completed_at: "2026-05-26T10:02:05Z",
            duration_ms: 5_000,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: null,
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
      });
      store.rebuildChatEntries();

      expect(useWorkspaceStore.getState().chatEntries).toEqual([
        expect.objectContaining({
          id: "timeline_node_decision:timeline-anchor",
          type: "stage_change",
          role: "system",
          content: "Review Decision Round 1 · 已选择返修",
          node_id: "timeline_node_decision",
        }),
      ]);
    },
  );

  it("rebuilds provider prompt events from spec node prompt snapshots", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_story_prompt",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_author",
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
      active_node_id: "timeline_node_author",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_author: makeNodeDetail({
          node_id: "timeline_node_author",
          node_type: "author_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          prompt: "[user]: 实现爬楼梯 Story Spec",
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "timeline_node_author:provider-prompt",
        type: "execution_event",
        role: "author",
        content: "Story 生成 · Provider Prompt · 24 字符",
        content_ref: { kind: "provider_prompt", nodeId: "timeline_node_author" },
        content_size: 24,
        has_full_content: true,
        metadata: expect.objectContaining({
          title: "Provider Prompt",
          provider: "claude_code",
        }),
      }),
    ]);
  });
});
