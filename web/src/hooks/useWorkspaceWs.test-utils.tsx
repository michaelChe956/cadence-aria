import { render } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import type { WorkItemPlanCandidateDto } from "../api/types";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { useWorkspaceWs } from "./useWorkspaceWs";

export function makeWorkItemPlanCandidate(
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

export function makeOutlineArtifactPayload() {
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

export function makeDraftArtifactPayload() {
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

export function makeBatchArtifactPayload() {
  return {
    batch_id: "batch_001",
    generation_round_id: "round_001",
    queue: ["outline_backend"],
    draft_records: [makeDraftArtifactPayload().draft_record],
    batch_status: "completed",
    failure_summary: [],
  };
}

export function makeCompileArtifactPayload() {
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

export class MockWebSocket {
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

export function renderWorkspaceHook(sessionId = "session_001") {
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

export function installWorkspaceWsTestHooks() {
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
}
