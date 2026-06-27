import { beforeEach } from "vitest";
import type { NodeDetail, WorkItemPlanCandidateDto } from "../api/types";
import { useWorkspaceStore } from "./workspace-ws-store";

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

export function makeContextBlockerArtifactPayload() {
  return {
    context_blockers: [],
    design_context_gaps: [],
    exploration_summary:
      "Outline 自动重跑后仍校验失败，已停止继续生成。主要问题：duplicate_outline_id - outline id outline_backend_session is duplicated。请终止当前流程并重新创建 Work Item Plan。",
    allowed_actions: ["provide_context", "abort"],
  };
}

export function makeNodeDetail(overrides: Partial<NodeDetail> = {}): NodeDetail {
  return {
    node_id: "timeline_node_001",
    session_id: "session_001",
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


export function installWorkspaceStoreTestHooks() {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });
}
