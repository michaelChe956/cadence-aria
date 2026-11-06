import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LifecycleCard, LifecycleColumns } from "../../api/types/lifecycle";
import type {
  IssueLifecycleGenerationActions,
  IssueLifecycleGenerationOptions,
} from "./useIssueLifecycleGeneration";
import { useIssueLifecycleGeneration } from "./useIssueLifecycleGeneration";
import type { WorkItemPlanOptionsFormValue } from "./WorkItemPlanOptionsDialog";

// REQ-WIGA-VOID-01 前端段（tasks 5.3）：作废态（随工作项组删除作废）下的
// 409 文案分流 SHALL NOT 出现 rebind 文案——旧登记已作废，无「与终止记录
// 一致」的约束，正确文案是「重新开启即可自由选择 provider」；非作废态
//（terminated 未作废）的既有 rebind 分流逐字不变。

const STORY_CARD: LifecycleCard = {
  kind: "story_spec",
  id: "story_0001",
  issueId: "issue_0001",
  title: "story",
  status: "confirmed",
  version: 1,
  preview: null,
  sourceIds: [],
  artifactVersions: [],
  raw: {
    story_spec_id: "story_0001",
    issue_id: "issue_0001",
    repository_id: "repo_0001",
    title: "story",
    current_version: 3,
    current_markdown_preview: null,
    confirmation_status: "confirmed",
    artifact_versions: [],
  },
};

const DESIGN_CARD: LifecycleCard = {
  kind: "design_spec",
  id: "design_0001",
  issueId: "issue_0001",
  title: "design",
  status: "confirmed",
  version: 2,
  preview: null,
  sourceIds: ["story_0001"],
  artifactVersions: [],
  raw: {
    design_spec_id: "design_0001",
    issue_id: "issue_0001",
    story_spec_ids: ["story_0001"],
    title: "design",
    current_version: 2,
    current_markdown_preview: null,
    confirmation_status: "confirmed",
    artifact_versions: [],
  },
};

const COLUMNS: LifecycleColumns = {
  issue: [],
  story_spec: [STORY_CARD],
  design_spec: [DESIGN_CARD],
  work_item: [],
};

const DIALOG_SELECTION_KEY = "selection_0001";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const LOGICAL_TARGET = {
  kind: "logical_codebase",
  logical_codebase_id: "11111111-1111-1111-1111-111111111111",
  logical_repository_id: "00000000-0000-0000-0000-000000000001",
} as const;

/** 已终止（未作废）：source 与当前 card 一致（等 payload 重新开启语义）。 */
function terminatedEnrollment(): Record<string, unknown> {
  return {
    enrollment_id: "enrollment_0001",
    selection_key: "selection_old",
    project_id: "project_0001",
    issue_id: "issue_0001",
    enabled: false,
    policy_revision: 3,
    source: {
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
    },
    options: {
      author_provider: "claude_code",
      reviewer_provider: "codex",
      review_rounds: 2,
      superpowers_enabled: false,
      openspec_enabled: true,
      plan_approval: "auto",
      plan_options: {
        include_integration_tests: false,
        include_e2e_tests: true,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
      },
    },
    target: LOGICAL_TARGET,
    prepare_intent_id: "prepare_intent_0001",
    plan_id: null,
    session_id: null,
    created_at: "2026-10-16T00:00:00Z",
    updated_at: "2026-11-06T00:00:00Z",
    terminated_at: "2026-11-06T00:00:00Z",
  };
}

/** 已作废（REQ-WIGA-VOID-01）：随工作项组删除作废，终止维度并存。 */
function voidedEnrollment(): Record<string, unknown> {
  return {
    ...terminatedEnrollment(),
    voided_at: "2026-11-06T01:00:00Z",
    void_reason: "work_item_group_deleted",
  };
}

/** 旧冲突码 409（automation_enrollment_conflict）的 PUT 失败注入。 */
function stubLifecycleFetch(
  enrollment: Record<string, unknown>,
  putFailure?: { code: string; message: string; status: number },
): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/automation-target")) {
        return jsonResponse({
          enrollment_target: LOGICAL_TARGET,
          resolved_options: {
            author_provider: "codex",
            reviewer_provider: "claude_code",
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            plan_approval: "manual",
            plan_options: {
              include_integration_tests: true,
              include_e2e_tests: false,
              force_frontend_backend_split: true,
              require_execution_plan_confirm: false,
            },
          },
        });
      }
      if (url.includes("/automation-enrollment")) {
        if (init?.method === "PUT" && putFailure) {
          return jsonResponse(
            { code: putFailure.code, message: putFailure.message, details: {} },
            putFailure.status,
          );
        }
        return jsonResponse(enrollment);
      }
      throw new Error(`unexpected fetch: ${url}`);
    }),
  );
}

const AUTOMATIC_OPTIONS: WorkItemPlanOptionsFormValue = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
  automation_mode: "automatic",
};

function renderGeneration(selectionKey: string): {
  actions: () => IssueLifecycleGenerationActions;
} {
  const { result } = renderHook((props: { selectionKey: string }) =>
    useIssueLifecycleGeneration({
      selectedProjectId: "project_0001",
      selectedColumns: COLUMNS,
      pendingWorkItemPlanLaunch: {
        card: DESIGN_CARD,
        options: AUTOMATIC_OPTIONS,
        selectionKey: props.selectionKey,
      },
      setPendingWorkItemPlanLaunch: vi.fn(),
      setError: vi.fn(),
      setSelectedCardKey: vi.fn(),
      refresh: vi.fn(async () => undefined),
      openDrawer: vi.fn(),
      onOpenWorkspace: vi.fn(),
    } satisfies IssueLifecycleGenerationOptions),
    { initialProps: { selectionKey } },
  );
  return { actions: () => result.current };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

const CONFLICT_409 = {
  code: "automation_enrollment_conflict",
  message: "automation enrollment revision conflict",
  status: 409,
} as const;

describe("useIssueLifecycleGeneration：作废态的 409 文案分流", () => {
  it("never surfaces the rebind hint on a voided enrollment", async () => {
    stubLifecycleFetch(voidedEnrollment(), CONFLICT_409);
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    // REQ-WIGA-VOID-01：旧登记已随工作项组删除作废——409 不再指引
    // 「换代 / 重新绑定（rebind）」；正确文案是重新开启即可自由选择 provider。
    let failure: unknown;
    try {
      await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);
    } catch (reason) {
      failure = reason;
    }
    expect(failure).toBeInstanceOf(Error);
    const message = String((failure as Error).message);
    expect(message).toMatch(/重新开启即可自由选择 provider/);
    expect(message).not.toMatch(/rebind|重绑|换代/);
  });

  it("keeps the existing rebind triage for a terminated-only enrollment", async () => {
    stubLifecycleFetch(terminatedEnrollment(), CONFLICT_409);
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    // 非作废态的既有分流逐字不变：旧冲突码在重新开启路径仍提示换代/重绑。
    await expect(
      actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS),
    ).rejects.toThrow(/换代|重绑|rebind/);
  });
});
