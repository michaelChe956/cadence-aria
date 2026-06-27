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

describe("workspace ws store artifact payloads", () => {
  installWorkspaceStoreTestHooks();

  it("resolves the latest unresolved gate prompt entry", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "gate-1",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认 1",
      timestamp: "2026-05-21T10:00:00Z",
    });
    store.appendChatEntry({
      id: "gate-2",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认 2",
      timestamp: "2026-05-21T10:01:00Z",
    });

    store.resolveGateEntry("request-change");
    const firstResolution = useWorkspaceStore.getState().chatEntries;
    expect(firstResolution[0]).toEqual(expect.objectContaining({ id: "gate-1" }));
    expect(firstResolution[0]).not.toHaveProperty("resolved");
    expect(firstResolution[1]).toEqual(
      expect.objectContaining({
        id: "gate-2",
        resolved: true,
        resolution: "request-change",
      }),
    );

    store.resolveGateEntry("confirm");
    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "gate-1",
        resolved: true,
        resolution: "confirm",
      }),
      expect.objectContaining({
        id: "gate-2",
        resolved: true,
        resolution: "request-change",
      }),
    ]);
  });

  it("sets workItemPlanCandidate from a session state with candidate artifact", () => {
    const store = useWorkspaceStore.getState();
    const candidate = makeWorkItemPlanCandidate();

    store.setSessionState({
      session_id: "session_candidate",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { candidate },
      providers: { author: "claude_code", reviewer: null },
    });

    expect(useWorkspaceStore.getState().workItemPlanCandidate).toEqual(candidate);
    expect(useWorkspaceStore.getState().artifact).toBeNull();
  });

  it("sets markdown artifact from a session state with markdown artifact", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_markdown",
      workspace_type: "story",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { markdown: "# Story", diff: null },
      providers: { author: "claude_code", reviewer: null },
    });

    expect(useWorkspaceStore.getState().artifact).toBe("# Story");
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
  });

  it("supports legacy string artifact in session state", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_legacy",
      workspace_type: "story",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: "# Legacy Story",
      providers: { author: "claude_code", reviewer: null },
    });

    expect(useWorkspaceStore.getState().artifact).toBe("# Legacy Story");
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
  });

  it("updates workItemPlanCandidate via setWorkItemPlanCandidate", () => {
    const store = useWorkspaceStore.getState();
    const candidate = makeWorkItemPlanCandidate();

    store.setWorkItemPlanCandidate(candidate);

    expect(useWorkspaceStore.getState().workItemPlanCandidate).toEqual(candidate);
  });

  it("stores work item plan outline payload from session state", () => {
    const store = useWorkspaceStore.getState();
    const outlineCandidate = makeOutlineArtifactPayload();

    store.setSessionState({
      session_id: "session_outline_artifact",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { outline_candidate: outlineCandidate } as never,
      providers: { author: "claude_code", reviewer: null },
    });

    expect((useWorkspaceStore.getState() as never as { workItemPlanArtifact: unknown }).workItemPlanArtifact).toEqual({
      type: "outline_candidate",
      payload: outlineCandidate,
    });
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
    expect(useWorkspaceStore.getState().artifact).toBeNull();
  });

  it("stores draft payload without clearing artifact history", () => {
    const store = useWorkspaceStore.getState();
    const draftCandidate = makeDraftArtifactPayload();

    store.setSessionState({
      session_id: "session_draft_artifact",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { draft_candidate: draftCandidate } as never,
      providers: { author: "claude_code", reviewer: null },
      artifact_version_summaries: [
        {
          version: 1,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-23T00:00:00Z",
          source_node_id: "node_draft",
        },
      ],
    });

    expect((useWorkspaceStore.getState() as never as { workItemPlanArtifact: unknown }).workItemPlanArtifact).toEqual({
      type: "draft_candidate",
      payload: draftCandidate,
    });
    expect(useWorkspaceStore.getState().artifactVersions).toHaveLength(1);
  });

  it("restores all work item plan artifact versions from summaries and attaches the current typed artifact", () => {
    const store = useWorkspaceStore.getState();
    const compileReport = makeCompileArtifactPayload();

    store.setSessionState({
      session_id: "session_work_item_plan_history",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: { compile_report: compileReport } as never,
      providers: { author: "claude_code", reviewer: "codex" },
      active_node_id: "node_compile",
      artifact_versions: [],
      artifact_version_summaries: [
        {
          version: 10,
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: "user",
          is_current: false,
          created_at: "2026-06-26T10:00:00Z",
          source_node_id: "node_outline",
        },
        {
          version: 11,
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: "user",
          is_current: false,
          created_at: "2026-06-26T10:01:00Z",
          source_node_id: "node_draft",
        },
        {
          version: 12,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-26T10:02:00Z",
          source_node_id: "node_compile",
        },
      ],
    });

    expect(useWorkspaceStore.getState().artifactVersions).toHaveLength(3);
    expect(useWorkspaceStore.getState().workItemPlanArtifactVersions).toMatchObject([
      { version: 10, artifact: null },
      { version: 11, artifact: null },
      {
        version: 12,
        artifact: { type: "compile_report", payload: compileReport },
      },
    ]);
  });

  it("rebuilds staged draft artifact updates with business labels", () => {
    const store = useWorkspaceStore.getState();
    const draftCandidate = makeDraftArtifactPayload();

    store.setSessionState({
      session_id: "session_draft_artifact_rebuild",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { draft_candidate: draftCandidate } as never,
      providers: { author: "claude_code", reviewer: null },
      active_node_id: "node_draft",
      timeline_nodes: [
        {
          node_id: "node_draft",
          node_type: "work_item_draft_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "completed",
          title: "Work Item Draft 生成",
          summary: null,
          started_at: "2026-06-23T00:00:00Z",
          completed_at: "2026-06-23T00:01:00Z",
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: null,
            review_rounds: 0,
          },
        },
      ],
      timeline_node_details: {
        node_draft: makeNodeDetail({
          node_id: "node_draft",
          node_type: "work_item_draft_run",
        }),
      },
      artifact_version_summaries: [
        {
          version: 4,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-23T00:01:00Z",
          source_node_id: "node_draft",
        },
      ],
    });

    const artifactEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "artifact_update");

    expect(artifactEntry).toMatchObject({
      type: "artifact_update",
      content: "Draft 已更新 · outline_backend · draft_backend_001",
      metadata: expect.objectContaining({
        version: 4,
        version_label: "内部版本 v4",
        artifact_label: "Draft",
        object_id: "outline_backend",
        draft_id: "draft_backend_001",
      }),
    });
    expect(artifactEntry?.content).not.toContain("-> v4");
  });

  it("stores compile report payload from session state", () => {
    const store = useWorkspaceStore.getState();
    const compileReport = makeCompileArtifactPayload();

    store.setSessionState({
      session_id: "session_compile_artifact",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: { compile_report: compileReport } as never,
      providers: { author: "claude_code", reviewer: null },
    });

    expect((useWorkspaceStore.getState() as never as { workItemPlanArtifact: unknown }).workItemPlanArtifact).toEqual({
      type: "compile_report",
      payload: compileReport,
    });
    expect(useWorkspaceStore.getState().artifact).toBeNull();
  });

  it("uses work item plan context blocker summary for the human confirm gate prompt", () => {
    const contextBlocker = makeContextBlockerArtifactPayload();

    useWorkspaceStore.getState().setSessionState({
      session_id: "session_outline_blocker",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: { context_blocker: contextBlocker } as never,
      providers: { author: "claude_code", reviewer: null },
      active_node_id: "node_context_blocker",
      timeline_nodes: [
        {
          node_id: "node_context_blocker",
          node_type: "work_item_plan_context_blocker",
          agent: null,
          stage: "human_confirm",
          round: null,
          status: "active",
          title: "WorkItemPlan 上下文确认",
          summary: "Outline 校验失败，请终止后重新创建 Work Item Plan",
          started_at: "2026-06-23T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: null,
            review_rounds: 0,
          },
        },
      ],
    });

    const gatePrompt = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "gate_prompt");
    expect(gatePrompt).toMatchObject({
      content: contextBlocker.exploration_summary,
    });
  });

  it("tracks typed work item plan artifact versions", () => {
    const store = useWorkspaceStore.getState();
    const outlineCandidate = makeOutlineArtifactPayload();

    store.setWorkItemPlanArtifact(
      {
        type: "outline_candidate",
        payload: outlineCandidate,
      },
      7,
    );

    expect(
      (useWorkspaceStore.getState() as never as { workItemPlanArtifactVersions: unknown[] })
        .workItemPlanArtifactVersions,
    ).toEqual([
      expect.objectContaining({
        version: 7,
        artifact: {
          type: "outline_candidate",
          payload: outlineCandidate,
        },
      }),
    ]);
  });

  it("keeps unknown work item plan node types available for fallback rendering", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_unknown_node",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      active_node_id: "node_future",
      timeline_nodes: [
        {
          node_id: "node_future",
          node_type: "work_item_plan_future_phase",
          agent: null,
          stage: "human_confirm",
          round: null,
          status: "active",
          title: "Future phase",
          summary: null,
          started_at: "2026-06-23T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: null,
            review_rounds: 0,
          },
        } as never,
      ],
    });

    expect(useWorkspaceStore.getState().activeNodeId).toBe("node_future");
    expect(useWorkspaceStore.getState().timelineNodes[0]?.node_type).toBe(
      "work_item_plan_future_phase",
    );
    expect(useWorkspaceStore.getState().nodeDetails.node_future?.node_type).toBe(
      "work_item_plan_future_phase",
    );
  });
});
