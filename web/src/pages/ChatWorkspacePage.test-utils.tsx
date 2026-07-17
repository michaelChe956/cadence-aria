import { beforeEach, vi } from "vitest";
import type { NodeDetail } from "../api/types";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type { ChatEntry } from "../state/chat-entries";
import { useWorkspaceStore, type TimelineNode } from "../state/workspace-ws-store";

type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

export function mockWorkspaceWs(overrides: Partial<WorkspaceWsApi> = {}) {
  const api: WorkspaceWsApi = {
    sendMessage: vi.fn(),
    sendContextNote: vi.fn(),
    sendStartGeneration: vi.fn(),
    retryInterruptedRun: vi.fn(() => true),
    sendSelectRevisionPath: vi.fn(),
    sendAuthorDecision: vi.fn(),
    sendRequestRevision: vi.fn(),
    sendRevertWorkItem: vi.fn(),
    sendSelectWorkItemGenerationMode: vi.fn(),
    sendRequestOutlineRevision: vi.fn(),
    sendWorkItemDraftDecision: vi.fn(),
    sendWorkItemBatchDecision: vi.fn(),
    sendWorkItemPlanCompileRecoveryAction: vi.fn(),
    sendHumanPresentationRevision: vi.fn(() => true),
    sendHumanConfirm: vi.fn(),
    sendHello: vi.fn(),
    sendPing: vi.fn(),
    startGeneration: vi.fn(),
    rollback: vi.fn(),
    confirm: vi.fn(),
    abort: vi.fn(),
    selectProvider: vi.fn(),
    sendProviderSelect: vi.fn(),
    sendReviewDecision: vi.fn(),
    respondPermission: vi.fn(),
    sendPermissionResponse: vi.fn(),
    sendChoiceResponse: vi.fn(),
    connectionStatus: "connected",
    isReconnecting: false,
    reconnectAttemptCount: 0,
    retryNow: vi.fn(),
    ...overrides,
  };
  vi.mocked(useWorkspaceWs).mockReturnValue(api);
  return api;
}


export function installChatWorkspacePageTestHooks() {
  beforeEach(() => {
    window.localStorage.clear();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    useWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });
}

export function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "reviewer_run",
    agent: "codex",
    stage: "cross_review",
    round: 1,
    status: "active",
    title: "Review Round 1",
    summary: "正在审核",
    started_at: "2026-05-20T00:00:00Z",
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

export function chatEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  return {
    id: "entry-1",
    type: "provider_stream",
    role: "reviewer",
    content: "review output",
    timestamp: "2026-05-20T00:00:00Z",
    ...overrides,
  };
}

export function makeNodeDetail(overrides: Partial<NodeDetail> = {}): NodeDetail {
  return {
    node_id: "timeline_node_001",
    session_id: "workspace_session_0001",
    node_type: "author_run",
    status: "completed",
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
    started_at: "2026-05-20T14:30:00Z",
    ended_at: null,
    ...overrides,
  };
}

export function workItemPlanCandidate(
  overrides: Partial<import("../api/types").WorkItemPlanCandidateDto> = {},
): import("../api/types").WorkItemPlanCandidateDto {
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
    work_items: [
      {
        candidate_id: "wi_001",
        title: "Frontend Auth",
        kind: "frontend",
        exclusive_write_scopes: ["src/auth"],
        depends_on: [],
        verification_plan_ref: null,
        meta: { summary: "前端登录" },
      },
    ],
    verification_plans: [],
    repository_profile: null,
    validator_findings: [],
    ...overrides,
  };
}

export function workItemPlanOutlinePayload() {
  return {
    outline: {
      id: "outline_version_001",
      plan_id: "plan_001",
      strategy_summary: "Split frontend and backend work.",
      work_items: [
        {
          outline_id: "outline_backend",
          title: "Backend flow",
          kind: "backend",
          sequence_hint: 1,
          depends_on_outline_ids: [],
          exclusive_write_scopes: ["src/product"],
          forbidden_write_scopes: [],
          context_budget: {
            target_context_k: "medium",
            max_summary_chars: 4000,
            max_handoff_chars: 2000,
            max_code_context_chars: 12000,
            max_context_file_refs: 12,
            max_traceability_refs: 12,
            max_dependency_handoffs: 4,
          },
          required_handoff_from_outline_ids: [],
          verification_strategy: "cargo test --locked",
          risk_notes: [],
        },
      ],
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

export function workItemDraftPayload(title = "Backend flow") {
  return {
    draft_record: {
      draft_id: "draft_backend_001",
      plan_id: "plan_001",
      generation_round_id: "round_001",
      outline_id: "outline_backend",
      batch_id: null,
      candidate: {
        outline_id: "outline_backend",
        title,
        kind: "backend",
        implementation_context: "Implement backend state transitions.",
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
        handoff_summary: "Backend state is ready for frontend.",
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

export function workItemBatchPayload(withFailure = false) {
  return {
    batch_id: "batch_001",
    generation_round_id: "round_001",
    queue: ["outline_backend"],
    draft_records: [workItemDraftPayload().draft_record],
    batch_status: "completed",
    failure_summary: withFailure
      ? [
          {
            draft_id: "draft_backend_001",
            outline_id: "outline_backend",
            status: "validation_failed",
          },
        ]
      : [],
  };
}

export function workItemCompileReportPayload(planCommitState: string) {
  return {
    compile_id: "compile_001",
    generation_round_id: "round_001",
    status: "recovery_required",
    plan_commit_state: planCommitState,
    work_item_ids: ["work_item_backend"],
    verification_plan_ids: ["verification_backend"],
    child_session_ids: ["session_child_backend"],
    validator_findings: [],
  };
}

export function workItemProjectionSessionArtifacts(missingRef = false) {
  const workItemProjection = {
    id: "projection-wi-01",
    work_item_revision_id: "revision-wi-01",
    canonical_contract_hash: "canonical-hash",
    projection_schema_version: 1,
    compiler_version: "projection-compiler-v1",
    human_projection: {
      logical_work_item_id: "WI-01",
      title: "初始化领域模型",
      goal: "建立初始化状态模型",
      non_goals: [],
      inputs: [],
      outputs: [],
      dependencies: [],
      scope_summary: { owned_scopes: [], forbidden_scopes: [] },
      completion_summary: [],
      source_refs: ["story:repository-init"],
      normative: false,
      used_by_provider: false,
    },
    coder_projection: {
      work_item_revision_id: "revision-wi-01",
      objective: "初始化状态模型并提交契约",
      required_input_contracts: [],
      task_refs: [],
      tasks: [],
      write_policy: { exclusive_scopes: [], forbidden_scopes: [] },
      acceptance_criteria: [],
      verification_checks: [],
      blocker_rules: [],
      handoff_contract: {
        required_fields: [],
        provided_contract_refs: [],
        reviewer_check_refs: [],
      },
    },
    reviewer_projection: {
      work_item_revision_id: "revision-wi-01",
      criterion_refs: [],
      requirement_matrix: [],
      scope_policy: { exclusive_scopes: [], forbidden_scopes: [] },
      input_contract_checks: [],
      output_contract_checks: [],
      verification_evidence_rules: [],
      blocker_routing: [],
    },
    human_projection_hash: "human-wi-hash",
    coder_projection_hash: "coder-wi-hash",
    reviewer_projection_hash: "reviewer-wi-hash",
    created_at: "2026-07-18T10:00:01Z",
  } as const;
  const planProjection = {
    id: "projection-plan-01",
    plan_revision_id: "plan-revision-01",
    dependency_graph_revision_id: "dependency-graph-01",
    work_item_projection_bundle_refs: [
      workItemProjection.id,
      ...(missingRef ? ["projection-wi-missing"] : []),
    ],
    human_group_projection: {
      plan_id: "plan-01",
      goal: "仓库初始化实时进度",
      split_reason: "按契约拆分",
      work_items: [
        {
          logical_work_item_id: "WI-01",
          title: "初始化领域模型",
          goal: "建立初始化状态模型",
          depends_on: [],
          provides: ["finalization-contract"],
          scope_summary: { owned_scopes: [], forbidden_scopes: [] },
        },
      ],
      contract_flow: [],
      risks: [],
      source_refs: ["story:repository-init"],
      normative: false,
      used_by_provider: false,
    },
    coder_group_context: {
      plan_id: "plan-01",
      ordered_logical_work_item_ids: ["WI-01"],
      dependency_edges: [],
      group_write_scopes: {},
    },
    reviewer_group_matrix: {
      plan_id: "plan-01",
      work_items: [],
      dependency_edges: [],
      design_traceability_refs: [],
    },
    human_group_projection_hash: "human-plan-hash",
    coder_group_context_hash: "coder-plan-hash",
    reviewer_group_matrix_hash: "reviewer-plan-hash",
    compiler_version: "projection-compiler-v1",
    created_at: "2026-07-18T10:00:00Z",
  } as const;
  const history = {
    entries: [
      {
        kind: "work_item_revision",
        id: "revision-wi-01",
        logical_work_item_id: "WI-01",
        related_revision_id: null,
        summary: "发布 Work Item revision",
        created_at: "2026-07-18T10:00:02Z",
      },
    ],
  } as const;
  const validation = { findings: [] } as const;
  const metadata = (version: number, isCurrent: boolean) => ({
    version,
    generated_by: "claude_code" as const,
    reviewed_by: "codex" as const,
    review_verdict: "pass" as const,
    confirmed_by: null,
    is_current: isCurrent,
    created_at: `2026-07-18T10:00:${String(version).padStart(2, "0")}Z`,
    source_node_id: "node-compile",
  });
  return {
    planProjection,
    workItemProjection,
    artifactVersions: [
      { ...metadata(20, false), work_item_projection: workItemProjection },
      { ...metadata(21, false), projection_validation: validation },
      { ...metadata(22, false), work_item_revision_history: history },
      { ...metadata(23, true), plan_projection: planProjection },
    ],
    artifactVersionSummaries: [
      metadata(20, false),
      metadata(21, false),
      metadata(22, false),
      metadata(23, true),
    ],
  };
}
