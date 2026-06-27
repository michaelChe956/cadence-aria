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

describe("workspace ws store base state", () => {
  installWorkspaceStoreTestHooks();

  it("clears partial streaming content when an active run is aborted", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("prepare_context");

    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("keeps streaming content while the stage remains running", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("running");

    expect(useWorkspaceStore.getState().streamingContent).toBe("partial output");
  });

  it("tracks stages visited by fast websocket transitions", () => {
    const store = useWorkspaceStore.getState();

    store.setStage("running");
    store.setStage("cross_review");
    store.setStage("human_confirm");

    expect(useWorkspaceStore.getState().visitedStages).toEqual([
      "prepare_context",
      "running",
      "author_confirm",
      "cross_review",
      "human_confirm",
    ]);
  });

  it("maps review decision and revision stages onto the cross review rail step", () => {
    const store = useWorkspaceStore.getState();

    store.setStage("running");
    store.setStage("cross_review");
    store.setStage("review_decision");
    store.setStage("revision");

    expect(useWorkspaceStore.getState().visitedStages).toEqual([
      "prepare_context",
      "running",
      "author_confirm",
      "cross_review",
    ]);
  });

  it("tracks and resolves pending permission requests", () => {
    const store = useWorkspaceStore.getState();
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(1);

    store.resolvePermissionRequest("perm_001");

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
  });

  it("marks permission request entries resolved when a response is sent", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "permission-request-1",
      type: "permission_request",
      role: "system",
      content: "shell · cargo test",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: { request_id: "perm_001" },
    });

    store.resolvePermissionRequest("perm_001", true);

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "permission-request-1",
        resolved: true,
        metadata: expect.objectContaining({ approved: true }),
      }),
      expect.objectContaining({
        type: "permission_response",
        role: "user",
        content: "已允许",
      }),
    ]);
  });

  it("marks choice request entries resolved and appends a choice response entry", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "choice-request-1",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [
          { id: "continue", label: "继续" },
          { id: "stop", label: "停止" },
        ],
      },
    } as ChatEntry);

    store.resolveChoiceRequest("choice_001", ["continue"], null);

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "choice-request-1",
        resolved: true,
        metadata: expect.objectContaining({
          response: { selected_option_ids: ["continue"], free_text: null },
        }),
      }),
      expect.objectContaining({
        type: "choice_response",
        role: "user",
        content: "已选择：继续",
      }),
    ]);
  });

  it("rejects stale choice requests and removes optimistic choice responses", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "choice-request-1",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [{ id: "continue", label: "继续" }],
      },
    } as ChatEntry);
    store.resolveChoiceRequest("choice_001", ["continue"], null);

    store.rejectChoiceRequest("choice_001", "ChoiceResponse id=choice_001 not found in pending");

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "choice-request-1",
        resolved: true,
        metadata: expect.objectContaining({
          rejected: true,
          rejection_reason: "ChoiceResponse id=choice_001 not found in pending",
        }),
      }),
    ]);
  });

  it("deduplicates pending permission requests by id", () => {
    const store = useWorkspaceStore.getState();

    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo clippy",
      risk_level: "high",
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toEqual([
      {
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo clippy",
        risk_level: "high",
      },
    ]);
  });

  it("updates provider status independently from workspace stage", () => {
    const store = useWorkspaceStore.getState();

    store.setProviderStatus("waiting_approval");

    expect(useWorkspaceStore.getState().providerStatus).toBe("waiting_approval");
    expect(useWorkspaceStore.getState().stage).toBe("prepare_context");
  });

  it("evicts content cache entries by byte budget", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      contentCache: emptyWorkspaceContentCache(6),
    });

    store.setContentCacheEntry("a", "aaa", 1);
    store.setContentCacheEntry("b", "bbb", 2);
    store.touchContentCacheEntry("a", 3);
    store.setContentCacheEntry("c", "ccc", 4);

    expect(workspaceContentCacheValues(useWorkspaceStore.getState().contentCache)).toEqual({
      a: "aaa",
      c: "ccc",
    });
  });

  it("merges hydrated node detail and rebuilds chat entries", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      timelineNodes: [
        {
          node_id: "node-1",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "completed",
          title: "Review Round 1",
          summary: "仅有可选建议",
          started_at: "2026-05-20T00:00:00Z",
          completed_at: "2026-05-20T00:01:00Z",
          duration_ms: 60_000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      nodeDetails: {
        "node-1": makeNodeDetail({
          node_id: "node-1",
          node_type: "reviewer_run",
          streaming_content: "summary only",
        }),
      },
    });

    store.setNodeDetail(
      makeNodeDetail({
        node_id: "node-1",
        node_type: "reviewer_run",
        streaming_content: "complete review output",
        verdict: {
          verdict: "needs_human",
          comments: "完整 comments",
          summary: "仅有可选建议",
          findings: [],
          review_gate: "user_confirm_allowed",
        },
      }),
    );

    expect(useWorkspaceStore.getState().nodeDetails["node-1"].streaming_content).toBe(
      "complete review output",
    );
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.content.includes("complete review output")),
    ).toBe(true);
  });

  it("rebuilds user triage gate prompts with review metadata from hydrated node detail", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "human_confirm",
      timelineNodes: [
        {
          node_id: "node-review-1",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "completed",
          title: "Review Round 1",
          summary: "返修意图需要人工判断",
          started_at: "2026-05-20T00:00:00Z",
          completed_at: "2026-05-20T00:01:00Z",
          duration_ms: 60_000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
        {
          node_id: "node-human-1",
          node_type: "human_confirm",
          agent: null,
          stage: "human_confirm",
          round: 1,
          status: "paused",
          title: "人工确认",
          summary: "等待用户裁决",
          started_at: "2026-05-20T00:01:00Z",
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
    });

    store.setNodeDetail(
      makeNodeDetail({
        node_id: "node-review-1",
        node_type: "reviewer_run",
        streaming_content: "Reviewer 要求返修但未输出 finding",
        verdict: {
          verdict: "needs_human",
          comments: "请补齐异常路径说明。",
          summary: "返修意图需要人工判断",
          findings: [
            {
              severity: "optional",
              message: "建议补充说明",
              evidence: "当前版本可用",
              impact: "不影响下一阶段",
              required_action: "补充说明段落",
            },
          ],
          review_gate: "user_triage_required",
        },
      }),
    );

    const gatePrompt = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "gate_prompt");
    expect(gatePrompt).toMatchObject({
      content: "需要人工确认",
      metadata: expect.objectContaining({
        comments: "请补齐异常路径说明。",
        review_gate: "user_triage_required",
        findings: [expect.objectContaining({ message: "建议补充说明" })],
      }),
    });
  });

  it("upserts execution events by id so command completion replaces running state", () => {
    const store = useWorkspaceStore.getState();

    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "started",
      title: "Command started",
      detail: null,
      command: "pwd",
      cwd: "/tmp/repo",
      output: null,
      exit_code: null,
    });
    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "completed",
      title: "Command completed",
      detail: "exit code 0",
      command: "pwd",
      cwd: "/tmp/repo",
      output: "/tmp/repo\n",
      exit_code: 0,
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
});
