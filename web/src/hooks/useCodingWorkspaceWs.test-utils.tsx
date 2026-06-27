import { render } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import type { CodingGateRequired, WorkItemExecutionPlan } from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useCodingWorkspaceWs } from "./useCodingWorkspaceWs";

export class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readonly sent: string[] = [];
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

type CodingWsApi = ReturnType<typeof useCodingWorkspaceWs>;

export function codingSessionState(overrides: Record<string, unknown> = {}) {
  return {
    type: "coding_session_state",
    attempt_id: "coding_attempt_0001",
    status: "running",
    stage: "testing",
    branch_name: "aria/work-items/work_item_0001/attempt-1",
    base_branch: "main",
    worktree_path: "/tmp/worktree",
    rework_count: 0,
    max_auto_rework: 2,
    head_commit: null,
    pushed_remote: null,
    role_provider_config_snapshot: {
      coder: "fake",
      tester: "fake",
      analyst: "fake",
      code_reviewer: "fake",
      internal_reviewer: "fake",
      review_rounds: 1,
      permission_modes: {
        coder: "supervised",
        tester: "auto",
        analyst: "auto",
        code_reviewer: "supervised",
        internal_reviewer: "supervised",
      },
    },
    provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
    chat_entries: [],
    timeline_nodes: [],
    active_node_id: null,
    testing_report: null,
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    pending_choices: [],
    ...overrides,
  };
}

export function blockedGate(overrides: Partial<CodingGateRequired> = {}): CodingGateRequired {
  return {
    gate_id: "gate_0001",
    kind: "blocked",
    title: "Review blocked",
    description: "Review payload parse failed",
    stage: "code_review",
    role: "code_reviewer",
    reason_code: "review_payload_parse_error",
    evidence_refs: ["code_review_0001.json"],
    raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt",
    available_actions: [
      {
        action_id: "retry_review",
        label: "重试审查",
        action_type: "retry_review",
      },
    ],
    ...overrides,
  };
}

export function executionPlan(
  overrides: Partial<WorkItemExecutionPlan> = {},
): WorkItemExecutionPlan {
  return {
    id: "work_item_execution_plan_0001",
    project_id: "project_0001",
    issue_id: "issue_0001",
    work_item_id: "work_item_0001",
    attempt_id: "coding_attempt_0001",
    status: "draft",
    goal: "实现后端 API",
    allowed_write_scopes: ["src/product/**"],
    forbidden_write_scopes: ["web/**"],
    dependency_handoffs: [],
    story_refs: ["story_spec_0001"],
    design_refs: ["design_spec_0001"],
    openspec_refs: ["REQ-001"],
    superpowers_contract: "use superpowers:test-driven-development",
    tdd_contract: "先写失败测试，再写实现",
    verification_plan_ref: "verification_plan_work_item_0001",
    verification_summary: "provider supplied required gate verify_backend_unit",
    risk_notes: [],
    created_at: "2026-06-16T00:00:00Z",
    updated_at: "2026-06-16T00:00:00Z",
    ...overrides,
  };
}

export function renderCodingHook(attemptId = "coding_attempt_0001") {
  let api: CodingWsApi | undefined;

  function Harness() {
    api = useCodingWorkspaceWs(attemptId);
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


export function installCodingWorkspaceWsTestHooks() {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    useCodingWorkspaceStore.getState().reset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });
}
