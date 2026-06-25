import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkItemPlanCandidateDto } from "../api/types";
import type { ChatEntry } from "../state/chat-entries";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { useWorkspaceWs } from "./useWorkspaceWs";

function makeWorkItemPlanCandidate(
  overrides: Partial<WorkItemPlanCandidateDto> = {},
): WorkItemPlanCandidateDto {
  return {
    plan: {
      plan_id: "plan_001",
      project_id: "project_001",
      issue_id: "issue_001",
      title: "Plan 001",
      source_story_spec_ids: [],
      source_design_spec_ids: [],
      options: {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
      },
      status: "draft",
      work_item_ids: [],
      repository_profile_ref: null,
      verification_plan_ids: [],
      dependency_graph: [],
      created_from_provider_run: null,
      validator_findings: [],
      review_summary: null,
      created_at: "2026-06-17T00:00:00Z",
      updated_at: "2026-06-17T00:00:00Z",
    },
    work_items: [],
    verification_plans: [],
    repository_profile: null,
    validator_findings: [],
    ...overrides,
  };
}

function makeOutlineArtifactPayload() {
  return {
    outline: {
      id: "outline_version_001",
      plan_id: "plan_001",
      strategy_summary: "Split frontend and backend work.",
      work_items: [],
      dependency_graph: [],
      risks: [],
      handoff_plan: [],
      created_at: "2026-06-23T00:00:00Z",
      updated_at: "2026-06-23T00:00:00Z",
    },
    design_context_gaps: [],
    validator_findings: [],
    context_blockers: [],
    current_generation_round_id: "round_001",
    selected_generation_mode: null,
  };
}

function makeDraftArtifactPayload() {
  return {
    draft_record: {
      draft_id: "draft_backend_001",
      plan_id: "plan_001",
      generation_round_id: "round_001",
      outline_id: "outline_backend",
      batch_id: null,
      candidate: {
        outline_id: "outline_backend",
        title: "Backend flow",
        kind: "backend",
        implementation_context: "Implement backend flow.",
        exclusive_write_scopes: ["src/product"],
        forbidden_write_scopes: [],
        depends_on_outline_ids: [],
        required_handoff_from_outline_ids: [],
        verification_plan: {
          commands: [],
          manual_checks: [],
          required_gates: [],
          risk_notes: [],
        },
        handoff_summary: "Backend handoff.",
      },
      status: "draft",
      active: true,
      superseded: false,
      superseded_by_draft_id: null,
      supersede_reason: null,
      copied_from_draft_id: null,
      generated_from_node_id: "node_draft",
      accepted_by_node_id: null,
      created_at: "2026-06-23T00:00:00Z",
      updated_at: "2026-06-23T00:00:00Z",
    },
    validator_findings: [],
    can_accept: true,
  };
}

function makeBatchArtifactPayload() {
  return {
    batch_id: "batch_001",
    generation_round_id: "round_001",
    queue: ["outline_backend"],
    draft_records: [makeDraftArtifactPayload().draft_record],
    batch_status: "completed",
    failure_summary: [],
  };
}

function makeCompileArtifactPayload() {
  return {
    compile_id: "compile_001",
    generation_round_id: "round_001",
    status: "committed",
    plan_commit_state: "committed",
    work_item_ids: ["work_item_backend"],
    verification_plan_ids: ["verification_backend"],
    child_session_ids: ["session_child_backend"],
    validator_findings: [],
  };
}

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readonly sent: string[] = [];
  readonly closeCodes: number[] = [];
  readonly url: string;
  readyState = MockWebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close(code = 1000) {
    this.closeCodes.push(code);
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.(new CloseEvent("close", { code }));
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.(new Event("open"));
  }

  receive(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(data) }));
  }
}

type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

function renderWorkspaceHook(sessionId = "session_001") {
  let api: WorkspaceWsApi | undefined;

  function Harness() {
    api = useWorkspaceWs(sessionId);
    return null;
  }

  const view = render(<Harness />);
  return {
    ...view,
    get api() {
      if (!api) throw new Error("hook not rendered");
      return api;
    },
    get ws() {
      const ws = MockWebSocket.instances[0];
      if (!ws) throw new Error("websocket not created");
      return ws;
    },
  };
}

describe("useWorkspaceWs", () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    useWorkspaceStore.getState().reset();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

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
