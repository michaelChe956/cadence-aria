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

describe("useWorkspaceWs outgoing actions", () => {
  installWorkspaceWsTestHooks();

  it("sends permission responses and resolves the pending request when connected", () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => undefined);
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.respondPermission("perm_001", true, " approved ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "permission_response",
        id: "perm_001",
        approved: true,
        reason: "approved",
      }),
    ]);
    expect(info).toHaveBeenCalledWith("[permission] sending response", {
      id: "perm_001",
      approved: true,
    });
    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
  });

  it("sends choice responses and resolves the pending choice when connected", () => {
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

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendChoiceResponse("choice_001", ["continue"], null);
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "choice_response",
        id: "choice_001",
        selected_option_ids: ["continue"],
        free_text: null,
      }),
    ]);
    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "choice_response",
      content: "已选择：继续",
    });
  });

  it("sends context notes when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendContextNote("补充上下文");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "context_note", content: "补充上下文" }),
    ]);
  });

  it("sends start generation with provider snapshot when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendStartGeneration(
        { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        true,
      );
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "start_generation",
        provider_config: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        reviewer_enabled: true,
      }),
    ]);
    expect(useWorkspaceStore.getState().providerStatus).toBe("running");
  });

  it("syncs provider selection locally after sending it", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.setState({
      providers: { author: "claude_code", reviewer: "codex" },
    });

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.selectProvider("author", "codex");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "provider_select", role: "author", provider: "codex" }),
    ]);
    expect(useWorkspaceStore.getState().providers).toEqual({
      author: "codex",
      reviewer: "codex",
    });
  });

  it("sends hello and ping lifecycle messages when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHello("session_001", "timeline_node_001");
      harness.api.sendPing();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: "timeline_node_001",
      }),
      JSON.stringify({ type: "ping" }),
    ]);
  });

  it("sends revision path decisions with trimmed optional context", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendSelectRevisionPath("revise-with-context", " 补充边界条件 ");
      harness.api.sendSelectRevisionPath("skip-to-human", "   ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "select_revision_path",
        path: "revise-with-context",
        extra_context: "补充边界条件",
      }),
      JSON.stringify({
        type: "select_revision_path",
        path: "skip-to-human",
        extra_context: null,
      }),
    ]);
  });

  it("sends human confirm decisions with nullable payload", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanConfirm("request-change", { reason: "需要补充" });
      harness.api.sendHumanConfirm("confirm");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "human_confirm",
        decision: "request-change",
        payload: { reason: "需要补充" },
      }),
      JSON.stringify({
        type: "human_confirm",
        decision: "confirm",
        payload: null,
      }),
    ]);
  });

  it("sends author decisions", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendAuthorDecision("accept");
      harness.api.sendAuthorDecision("reject");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "author_decision",
        decision: "accept",
      }),
      JSON.stringify({
        type: "author_decision",
        decision: "reject",
      }),
    ]);
  });

  it("marks the latest gate prompt resolved when a human confirm decision is sent", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().appendChatEntry({
      id: "gate-1",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-05-21T10:00:00Z",
    });

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanConfirm("terminate");
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "gate-1",
        resolved: true,
        resolution: "terminate",
      }),
    ]);
  });

  it("keeps deprecated sendMessage as a context note sender", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendMessage("旧入口上下文");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "context_note", content: "旧入口上下文" }),
    ]);
    expect(warn).toHaveBeenCalledWith(
      "sendMessage is deprecated, use sendContextNote or sendStartGeneration",
    );
  });

  it("keeps deprecated startGeneration as a warning-only no-op", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.startGeneration();
    });

    expect(harness.ws.sent).toHaveLength(0);
    expect(warn).toHaveBeenCalledWith("startGeneration() without args is deprecated");
  });

  it("sends hello automatically with the last active node when connected", () => {
    useWorkspaceStore.setState({ activeNodeId: "timeline_node_001" });
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: "timeline_node_001",
      }),
    ]);
  });

  it("sends ping every 25 seconds while connected", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
    });
    act(() => {
      vi.advanceTimersByTime(25_000);
    });

    expect(harness.ws.sent).toContain(JSON.stringify({ type: "ping" }));
  });

  it("closes stale sockets after 60 seconds without any server message", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
    });
    act(() => {
      vi.advanceTimersByTime(75_000);
    });

    expect(harness.ws.readyState).toBe(MockWebSocket.CLOSED);
    expect(harness.ws.closeCodes).toContain(4000);
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");
  });

  it("clears pending stream buffers when the socket closes before scheduled flush", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
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
      });
      harness.ws.receive({
        type: "stage_change",
        stage: "running",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "pending",
        node_id: "timeline_node_001",
      });
    });

    expect(useWorkspaceStore.getState().streamBuffers.timeline_node_001).toBeDefined();

    act(() => {
      harness.ws.close(1000);
      vi.advanceTimersByTime(80);
    });

    expect(useWorkspaceStore.getState().streamBuffers).toEqual({});
  });

  it("keeps the socket open during revision even when server messages are quiet", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.receive({ type: "stage_change", stage: "revision" });
    });
    act(() => {
      vi.advanceTimersByTime(75_000);
    });

    expect(harness.ws.readyState).toBe(MockWebSocket.OPEN);
    expect(useWorkspaceStore.getState().connectionStatus).toBe("connected");
  });

  it("opens a replacement websocket after an abnormal close", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(MockWebSocket.instances).toHaveLength(2);
    expect(MockWebSocket.instances[1].url).toBe(harness.ws.url);
  });

  it("keeps reconnecting after a replacement websocket errors", () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    const replacement = MockWebSocket.instances[1];
    expect(replacement).toBeDefined();
    act(() => {
      replacement.onerror?.(new Event("error"));
    });
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");

    act(() => {
      vi.advanceTimersByTime(2000);
    });

    expect(MockWebSocket.instances).toHaveLength(3);
  });

  it("keeps reconnecting when a replacement websocket stays connecting", () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(MockWebSocket.instances[1].readyState).toBe(MockWebSocket.CONNECTING);
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");

    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(MockWebSocket.instances).toHaveLength(3);
  });

  it("keeps pending permission requests when the socket is not open", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    act(() => {
      harness.api.respondPermission("perm_001", false, "denied");
    });

    expect(harness.ws.sent).toHaveLength(0);
    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(1);
  });

  it("stores work item plan candidate from artifact_update messages", () => {
    const harness = renderWorkspaceHook();
    const candidate = makeWorkItemPlanCandidate();

    act(() => {
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        candidate,
      });
    });

    expect(useWorkspaceStore.getState().workItemPlanCandidate).toEqual(candidate);
    expect(useWorkspaceStore.getState().artifact).toBeNull();
    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "artifact_update",
      content: "Work Item Plan 候选已更新 -> v1",
      metadata: { version: 1, candidate: true },
    });
  });

  it("stores markdown artifact from artifact_update messages and clears candidate", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        candidate: makeWorkItemPlanCandidate(),
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 2,
        markdown: "# Story",
        diff: null,
      });
    });

    expect(useWorkspaceStore.getState().artifact).toBe("# Story");
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
  });

  it("updates work item plan candidate on same version revert meta update", () => {
    const harness = renderWorkspaceHook();
    const initialCandidate = makeWorkItemPlanCandidate({
      work_items: [
        {
          candidate_id: "wi_001",
          title: "Item 1",
          kind: "frontend",
          exclusive_write_scopes: [],
          depends_on: [],
          verification_plan_ref: null,
          meta: { summary: "summary" },
        },
      ],
    });
    const revertedCandidate = makeWorkItemPlanCandidate({
      work_items: [
        {
          candidate_id: "wi_001",
          title: "Item 1",
          kind: "frontend",
          exclusive_write_scopes: [],
          depends_on: [],
          verification_plan_ref: null,
          meta: { summary: "summary" },
          reverted: true,
          revert_feedback: "范围过大",
        },
      ],
    });

    act(() => {
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        candidate: initialCandidate,
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        candidate: revertedCandidate,
      });
    });

    expect(useWorkspaceStore.getState().workItemPlanCandidate).toEqual(revertedCandidate);
  });

  it("stores staged work item plan artifact_update payloads", () => {
    const harness = renderWorkspaceHook();
    const outlineCandidate = makeOutlineArtifactPayload();
    const draftCandidate = makeDraftArtifactPayload();
    const batchState = makeBatchArtifactPayload();
    const compileReport = makeCompileArtifactPayload();

    act(() => {
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        outline_candidate: outlineCandidate,
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 2,
        draft_candidate: draftCandidate,
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 3,
        batch_state: batchState,
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 4,
        compile_report: compileReport,
      });
    });

    expect(useWorkspaceStore.getState().workItemPlanArtifact).toEqual({
      type: "compile_report",
      payload: compileReport,
    });
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
    expect(useWorkspaceStore.getState().artifact).toBeNull();
    const draftEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.metadata?.artifact_type === "draft_candidate");
    expect(draftEntry).toMatchObject({
      type: "artifact_update",
      content: "Draft 已更新 · outline_backend · draft_backend_001",
      metadata: expect.objectContaining({
        version: 2,
        version_label: "内部版本 v2",
        artifact_type: "draft_candidate",
        artifact_label: "Draft",
        object_id: "outline_backend",
        object_title: "Backend flow",
        draft_id: "draft_backend_001",
        status_label: "draft",
      }),
    });
    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "artifact_update",
      content: "Compile Report 已更新 · committed",
      metadata: {
        version: 4,
        version_label: "内部版本 v4",
        artifact_type: "compile_report",
        artifact_label: "Compile Report",
        object_id: "compile_001",
        status_label: "committed",
      },
    });
    expect(useWorkspaceStore.getState().chatEntries.at(-1)?.content).not.toContain("-> v4");
  });

  it("sends revert_work_item messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendRevertWorkItem("wi_001", " 范围过大 ", false);
      harness.api.sendRevertWorkItem("wi_001", undefined, true);
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "revert_work_item",
        work_item_id: "wi_001",
        feedback: "范围过大",
        clear: false,
      }),
      JSON.stringify({
        type: "revert_work_item",
        work_item_id: "wi_001",
        feedback: null,
        clear: true,
      }),
    ]);
  });

  it("sends request_revision messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendRequestRevision(" 请重新生成前端项 ");
      harness.api.sendRequestRevision();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "request_revision",
        feedback: {
          feedback_types: ["revision"],
          description: "请重新生成前端项",
        },
      }),
      JSON.stringify({
        type: "request_revision",
        feedback: {
          feedback_types: ["revision"],
          description: "",
        },
      }),
    ]);
  });

  it("sends staged work item plan workflow messages", () => {
    const harness = renderWorkspaceHook();
    const api = harness.api as unknown as {
      sendSelectWorkItemGenerationMode: (mode: "serial" | "batch") => void;
      sendRequestOutlineRevision: (feedback?: string) => void;
      sendWorkItemDraftDecision: (
        outlineId: string,
        decision: "accept" | "rewrite" | "pause",
        feedback?: string,
      ) => void;
      sendWorkItemBatchDecision: (
        decision: "accept_all" | "rewrite_batch" | "pause" | "downgrade_to_serial",
        feedback?: string,
        firstAffectedOutlineId?: string,
      ) => void;
      sendWorkItemPlanCompileRecoveryAction: (
        action: "continue" | "abort_and_rollback" | "human_triage",
        reason?: string,
      ) => void;
    };

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      api.sendSelectWorkItemGenerationMode("serial");
      api.sendRequestOutlineRevision(" 需要调整拆分 ");
      api.sendWorkItemDraftDecision("outline_backend", "rewrite", " 缩小范围 ");
      api.sendWorkItemBatchDecision("downgrade_to_serial", " 严格校验失败 ", "outline_backend");
      api.sendWorkItemPlanCompileRecoveryAction("human_triage", " 需要人工检查 ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "select_work_item_generation_mode", mode: "serial" }),
      JSON.stringify({
        type: "request_outline_revision",
        feedback: "需要调整拆分",
      }),
      JSON.stringify({
        type: "work_item_draft_decision",
        outline_id: "outline_backend",
        decision: "rewrite",
        feedback: "缩小范围",
      }),
      JSON.stringify({
        type: "work_item_batch_decision",
        decision: "downgrade_to_serial",
        feedback: "严格校验失败",
        first_affected_outline_id: "outline_backend",
      }),
      JSON.stringify({
        type: "work_item_plan_compile_recovery_action",
        action: "human_triage",
        reason: "需要人工检查",
      }),
    ]);
  });
});
