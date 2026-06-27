import { beforeEach, vi } from "vitest";
import type { WorkItemExecutionPlan } from "../api/types";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";

type CodingWsApi = ReturnType<typeof useCodingWorkspaceWs>;

export const DEFAULT_PERMISSION_MODES = {
  coder: "supervised",
  tester: "auto",
  analyst: "auto",
  code_reviewer: "supervised",
  internal_reviewer: "supervised",
} as const;

export function mockCodingWs(overrides: Partial<CodingWsApi> = {}) {
  const api: CodingWsApi = {
    startCoding: vi.fn(),
    sendContextNote: vi.fn(),
    sendProviderSelect: vi.fn(),
    sendPermissionModeSelect: vi.fn(),
    confirmStageGate: vi.fn(),
    respondPermission: vi.fn(),
    respondChoice: vi.fn(),
    respondGate: vi.fn(),
    continueRework: vi.fn(),
    finalConfirm: vi.fn(),
    abortAttempt: vi.fn(),
    requestManualPause: vi.fn(),
    sendHello: vi.fn(),
    sendPing: vi.fn(),
    ...overrides,
  };
  vi.mocked(useCodingWorkspaceWs).mockReturnValue(api);
  return api;
}

export function readyCodingState() {
  return {
    attemptId: "coding_attempt_0001",
    status: "created" as const,
    stage: "prepare_context" as const,
    branchName: "aria/work-items/work_item_0001/attempt-1",
    baseBranch: "main",
    worktreePath: "/tmp/worktree",
    timelineNodes: [],
    chatEntries: [],
  };
}

export function executionPlan(overrides: Partial<WorkItemExecutionPlan> = {}): WorkItemExecutionPlan {
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


export function installCodingWorkspacePageTestHooks() {
  beforeEach(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    useCodingWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });
}
