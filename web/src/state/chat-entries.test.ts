import { beforeEach, describe, expect, it } from "vitest";
import type { NodeDetail } from "../api/types";
import type { ChatEntry } from "./chat-entries";
import { useWorkspaceStore, type TimelineNode } from "./workspace-ws-store";

function makeTimelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-author-1",
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
    ...overrides,
  };
}

function makeNodeDetail(overrides: Partial<NodeDetail> = {}): NodeDetail {
  return {
    node_id: "node-author-1",
    session_id: "session-chat-1",
    node_type: "author_run",
    status: "active",
    agent_role: "author",
    provider: { name: "claude_code", model: "claude-opus-4" },
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
    ...overrides,
  };
}

describe("chat entries store", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("appends stream chunks to the active provider chat entry and finalizes it", () => {
    const store = useWorkspaceStore.getState();
    const entry: ChatEntry = {
      id: "stream-node-author-1",
      type: "provider_stream",
      role: "author",
      content: "",
      timestamp: "2026-05-21T10:00:00Z",
      node_id: "node-author-1",
    };

    store.appendChatEntry(entry);
    store.updateStreamingEntry(entry.id, "第一段");
    store.updateStreamingEntry(entry.id, "第二段");

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      {
        ...entry,
        content: "第一段第二段",
      },
    ]);
    expect(useWorkspaceStore.getState().activeStreamEntryId).toBe(entry.id);

    store.finalizeStreamingEntry(entry.id);

    expect(useWorkspaceStore.getState().activeStreamEntryId).toBeNull();
  });

  it("shows the prepared workspace context as the first user chat entry", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "session-prepared-context",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [
        {
          id: "msg_001",
          role: "system",
          content:
            "Workspace 生成任务已准备\n\n[system]\n你是候选 spec 生成器。\n\n[canonical_inputs]\nIssue: 爬楼梯问题",
          checkpoint_id: null,
          created_at: "2026-05-21T09:00:00Z",
        },
      ],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: null,
    });

    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      {
        id: "prepared-context:msg_001",
        type: "context_note",
        role: "user",
        content:
          "Workspace 生成任务已准备\n\n[system]\n你是候选 spec 生成器。\n\n[canonical_inputs]\nIssue: 爬楼梯问题",
        timestamp: "2026-05-21T09:00:00Z",
        metadata: { prepared: true },
      },
    ]);
  });

  it("updates prepared context provider guidance when author provider changes locally", () => {
    const store = useWorkspaceStore.getState();
    const preparedContext =
      "Workspace 生成任务已准备\n\n" +
      "[system]\n你是 Aria 的候选 spec 生成器。\n\n" +
      "[workflow_discipline]\n" +
      "必须遵守 using-superpowers 与 brainstorming 的纪律。\n" +
      "当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。\n\n" +
      "[output_schema]\nMarkdown Story Spec";

    store.setSessionState({
      session_id: "session-provider-guidance",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [
        {
          id: "msg_001",
          role: "system",
          content: preparedContext,
          checkpoint_id: null,
          created_at: "2026-05-21T09:00:00Z",
        },
      ],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: null,
    });

    store.setProviderSelection("author", "codex");

    const state = useWorkspaceStore.getState();
    expect(state.messages[0].content).toContain("当前 author provider 是 Codex");
    expect(state.messages[0].content).toContain("requestUserInput");
    expect(state.messages[0].content).not.toContain("当前 author provider 是 Claude Code");
    expect(state.messages[0].content).not.toContain("AskUserQuestion");
    expect(state.chatEntries[0].content).toBe(state.messages[0].content);
  });

  it("normalizes stale prepared context guidance from session state provider", () => {
    const store = useWorkspaceStore.getState();
    const staleContext =
      "Workspace 生成任务已准备\n\n" +
      "[system]\n你是 Aria 的候选 spec 生成器。\n\n" +
      "[workflow_discipline]\n" +
      "必须遵守 using-superpowers 与 brainstorming 的纪律。\n" +
      "当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。\n\n" +
      "[output_schema]\nMarkdown Story Spec";

    store.setSessionState({
      session_id: "session-provider-guidance-state",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [
        {
          id: "msg_001",
          role: "system",
          content: staleContext,
          checkpoint_id: null,
          created_at: "2026-05-21T09:00:00Z",
        },
      ],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: null,
    });

    const state = useWorkspaceStore.getState();
    expect(state.messages[0].content).toContain("当前 author provider 是 Codex");
    expect(state.messages[0].content).toContain("requestUserInput");
    expect(state.messages[0].content).not.toContain("当前 author provider 是 Claude Code");
    expect(state.messages[0].content).not.toContain("AskUserQuestion");
    expect(state.chatEntries[0].content).toBe(state.messages[0].content);
  });

  it("rebuilds chat entries from timeline node details in timeline order", () => {
    const contextNode = makeTimelineNode({
      node_id: "node-context-1",
      node_type: "context_note",
      agent: null,
      stage: "prepare_context",
      status: "completed",
      title: "补充上下文",
      summary: "需要支持手机号登录",
      started_at: "2026-05-21T10:00:00Z",
      completed_at: "2026-05-21T10:00:01Z",
    });
    const authorNode = makeTimelineNode({
      node_id: "node-author-1",
      node_type: "author_run",
      status: "completed",
      started_at: "2026-05-21T10:00:02Z",
      completed_at: "2026-05-21T10:01:00Z",
      artifact_ref: "artifact_current",
    });
    const reviewerNode = makeTimelineNode({
      node_id: "node-reviewer-1",
      node_type: "reviewer_run",
      agent: "codex",
      stage: "cross_review",
      status: "completed",
      title: "Review Round 1",
      started_at: "2026-05-21T10:01:01Z",
      completed_at: "2026-05-21T10:02:00Z",
    });
    const humanConfirmNode = makeTimelineNode({
      node_id: "node-human-1",
      node_type: "human_confirm",
      agent: null,
      stage: "human_confirm",
      status: "active",
      title: "人工确认",
      summary: "等待人工确认",
      started_at: "2026-05-21T10:02:01Z",
      completed_at: null,
    });

    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "session-chat-1",
      workspace_type: "story",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: "# Story",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [contextNode, authorNode, reviewerNode, humanConfirmNode],
      active_node_id: "node-human-1",
      artifact_versions: [
        {
          version: 1,
          markdown: "# Story",
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          created_at: "2026-05-21T10:01:00Z",
          source_node_id: "node-author-1",
        },
      ],
      timeline_node_details: {
        "node-context-1": makeNodeDetail({
          node_id: "node-context-1",
          node_type: "context_note",
          agent_role: null,
          provider: null,
          streaming_content: "需要支持手机号登录",
          started_at: "2026-05-21T10:00:00Z",
          ended_at: "2026-05-21T10:00:01Z",
        }),
        "node-author-1": makeNodeDetail({
          streaming_content: "## 功能需求\n\n支持手机号登录。",
          execution_events: [
            {
              event_id: "exec-1",
              node_id: "node-author-1",
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
          ],
          permission_events: [
            {
              request_id: "permission-1",
              request: {
                tool_name: "shell",
                description: "cargo test",
                risk_level: "medium",
              },
              response: { approved: true, reason: null },
              ts: "2026-05-21T10:00:20Z",
            },
          ],
          artifact_ref: { artifact_id: "artifact_current", version: 1 },
          ended_at: "2026-05-21T10:01:00Z",
        }),
        "node-reviewer-1": makeNodeDetail({
          node_id: "node-reviewer-1",
          node_type: "reviewer_run",
          agent_role: "reviewer",
          provider: { name: "codex", model: "gpt-5.4" },
          streaming_content: "审核通过。",
          verdict: {
            verdict: "pass",
            comments: "覆盖核心路径",
            summary: "可以进入人工确认",
          },
          started_at: "2026-05-21T10:01:01Z",
          ended_at: "2026-05-21T10:02:00Z",
        }),
      },
      active_run_id: null,
    });

    store.rebuildChatEntries();

    const entries = useWorkspaceStore.getState().chatEntries;
    expect(entries.map((entry) => entry.type)).toEqual([
      "context_note",
      "provider_stream",
      "execution_event",
      "permission_request",
      "permission_response",
      "artifact_update",
      "provider_stream",
      "review_verdict",
      "gate_prompt",
    ]);
    expect(entries[0]).toMatchObject({
      role: "user",
      content: "需要支持手机号登录",
      node_id: "node-context-1",
    });
    expect(entries[1]).toMatchObject({
      role: "author",
      content: "## 功能需求\n\n支持手机号登录。",
      node_id: "node-author-1",
    });
    expect(entries[3].metadata).toMatchObject({
      request_id: "permission-1",
      risk_level: "medium",
    });
    expect(entries[4]).toMatchObject({
      role: "user",
      content: "已允许 shell",
    });
    expect(entries[5]).toMatchObject({
      content: "产物已更新 -> v1",
      node_id: "node-author-1",
    });
    expect(entries[7]).toMatchObject({
      role: "reviewer",
      content: "可以进入人工确认",
    });
    expect(entries[8]).toMatchObject({
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      node_id: "node-human-1",
    });
  });

  it("collapses failed provider attempts when a timeline node is an automatic retry", () => {
    const failedAttempt = makeTimelineNode({
      node_id: "timeline_node_006",
      node_type: "work_item_plan_outline_run",
      agent: "codex",
      status: "failed",
      title: "WorkItemPlan Outline 生成",
      started_at: "2026-06-24T10:00:00Z",
    });
    const retryAttempt = makeTimelineNode({
      node_id: "timeline_node_007",
      node_type: "work_item_plan_outline_run",
      agent: "codex",
      status: "completed",
      title: "WorkItemPlan Outline 生成",
      started_at: "2026-06-24T10:01:00Z",
      retry: {
        retry_of_node_id: "timeline_node_006",
        retry_attempt: 2,
        retry_reason: "outline_structured_output_parse_error",
        retry_error: {
          code: "outline_structured_output_parse_error",
          message:
            "Provider did not return a valid WorkItemPlan Outline structured output: invalid structured output json",
        },
      },
    });

    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "session-work-item-plan",
      workspace_type: "work_item",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "codex" },
      timeline_nodes: [failedAttempt, retryAttempt],
      active_node_id: "timeline_node_007",
      artifact_versions: [],
      timeline_node_details: {
        "timeline_node_006": makeNodeDetail({
          node_id: "timeline_node_006",
          node_type: "work_item_plan_outline_run",
          provider: { name: "codex", model: "gpt-5" },
          streaming_content: "错误的第一轮 author 输出",
          status: "failed",
        }),
        "timeline_node_007": makeNodeDetail({
          node_id: "timeline_node_007",
          node_type: "work_item_plan_outline_run",
          provider: { name: "codex", model: "gpt-5" },
          streaming_content: "修正后的完整 outline",
          status: "completed",
        }),
      },
      active_run_id: null,
    });

    store.rebuildChatEntries();

    const providerEntries = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "provider_stream");
    expect(providerEntries).toHaveLength(1);
    expect(providerEntries[0]).toMatchObject({
      node_id: "timeline_node_007",
      content: "修正后的完整 outline",
      metadata: {
        provider: "codex",
        retry: {
          retry_of_node_id: "timeline_node_006",
          retry_attempt: 2,
          retry_reason: "outline_structured_output_parse_error",
          retry_error: {
            code: "outline_structured_output_parse_error",
          },
        },
      },
    });
    expect(providerEntries[0].metadata?.retry).toMatchObject({
      retry_error: {
        message: expect.stringContaining("Provider did not return a valid WorkItemPlan Outline"),
      },
    });
  });

  it("removes the visible source attempt when an automatic retry node arrives live", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "timeline_node_006:stream",
      type: "provider_stream",
      role: "author",
      content: "错误的第一轮 author 输出",
      timestamp: "2026-06-24T10:00:00Z",
      node_id: "timeline_node_006",
      metadata: { provider: "codex" },
    });

    store.addTimelineNode(
      makeTimelineNode({
        node_id: "timeline_node_007",
        node_type: "work_item_plan_outline_run",
        agent: "codex",
        status: "active",
        title: "WorkItemPlan Outline 生成",
        retry: {
          retry_of_node_id: "timeline_node_006",
          retry_attempt: 2,
          retry_reason: "outline_structured_output_parse_error",
          retry_error: {
            code: "outline_structured_output_parse_error",
            message: "Provider did not return a valid WorkItemPlan Outline structured output",
          },
        },
      }),
    );
    store.appendBufferedStreamChunk("修正后的完整 outline", "timeline_node_007", "author");
    store.flushBufferedStream("timeline_node_007");

    const entries = useWorkspaceStore.getState().chatEntries;
    expect(entries.map((entry) => entry.node_id)).toEqual(["timeline_node_007"]);
    expect(entries[0]).toMatchObject({
      content: "修正后的完整 outline",
      metadata: {
        provider: "codex",
        retry: {
          retry_of_node_id: "timeline_node_006",
          retry_attempt: 2,
        },
      },
    });
  });
});
