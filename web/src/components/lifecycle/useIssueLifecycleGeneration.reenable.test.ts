import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import type { WorkItemPlanOptionsFormValue } from "./WorkItemPlanOptionsDialog";
import type { IssueLifecycleGenerationActions } from "./useIssueLifecycleGeneration";
import { useIssueLifecycleGeneration } from "./useIssueLifecycleGeneration";

// 批 2 T4（REQ-WIGA-09 修订）前端段：terminated 的 enrollment 上人工
// 「启用自动化」＝显式重新开启。契约（owner 裁定 + 主规格 delta）：
//   ① Enable 必须携带稳定 command_id（同一动作重试复用同一 id；弹窗重开
//      或换参数换新 id）——缺它后端 422；
//   ② 重新开启的 payload（source/options/target）与终止记录逐字段相等，
//      不顺便改写授权——服务端逐字段比对，改了即 409；
//   ③ expected_revision 取终止记录当前 revision（线性化延续）；
//   ④ 收口未完成（409 automation_plan_cleanup_required）提示先重试清理，
//      不呈现通用冲突文案；
//   ⑤ 非 terminated 的常规并发语义不变。

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

// 镜像真实后端 GET automation-target 投影 wire。
function automationTargetResponse(url: string): unknown {
  const query = new URL(url, "http://localhost").searchParams;
  return {
    enrollment_target: {
      kind: "single_repository",
      repository_id: "00000000-0000-0000-0000-000000000001",
    },
    resolved_options: {
      author_provider: query.get("author_provider") ?? "codex",
      reviewer_provider: query.get("reviewer_provider") ?? "claude_code",
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      plan_approval: "manual",
      plan_options: {
        include_integration_tests: query.get("include_integration_tests") !== "false",
        include_e2e_tests: query.get("include_e2e_tests") === "true",
        force_frontend_backend_split:
          query.get("force_frontend_backend_split") === "true",
        require_execution_plan_confirm:
          query.get("require_execution_plan_confirm") === "true",
      },
    },
  };
}

type EnableCommandBody = {
  expected_revision: number | null;
  command: {
    type: string;
    /** 契约要求必带（缺它后端 422）；缺失时断言即红。 */
    command_id: string;
    selection_key: string;
    source: Record<string, unknown>;
    options: Record<string, unknown>;
    target?: Record<string, unknown>;
  };
};

type CapturedRequests = {
  enableBodies: EnableCommandBody[];
  targetGets: string[];
};

/** 已终止的 enrollment 投影（terminated_at 非空即终态事实）。 */
function terminatedEnrollment(): Record<string, unknown> {
  return {
    enrollment_id: "enrollment_0001",
    selection_key: "selection_old",
    project_id: "project_0001",
    issue_id: "issue_0001",
    enabled: false,
    policy_revision: 3,
    source: {
      stories: [{ id: "story_0001", version: 2 }],
      designs: [{ id: "design_0001", version: 1 }],
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
    target: {
      kind: "single_repository",
      repository_id: "00000000-0000-0000-0000-000000000001",
    },
    prepare_intent_id: "prepare_intent_0001",
    plan_id: null,
    session_id: null,
    created_at: "2026-10-16T00:00:00Z",
    updated_at: "2026-10-20T00:00:00Z",
    terminated_at: "2026-10-20T00:00:00Z",
  };
}

function stubLifecycleFetch(
  enrollment: Record<string, unknown> | null,
  putFailure?: { code: string; message: string; status: number },
): CapturedRequests {
  const captured: CapturedRequests = { enableBodies: [], targetGets: [] };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/automation-target")) {
        captured.targetGets.push(url);
        return jsonResponse(automationTargetResponse(url));
      }
      if (url.includes("/automation-enrollment")) {
        if (init?.method === "PUT") {
          const body = JSON.parse(String(init.body)) as EnableCommandBody;
          captured.enableBodies.push(body);
          if (putFailure) {
            return jsonResponse(
              { code: putFailure.code, message: putFailure.message, details: {} },
              putFailure.status,
            );
          }
          const command = body.command;
          return jsonResponse({
            enrollment_id: "enrollment_0001",
            selection_key: command.selection_key,
            project_id: "project_0001",
            issue_id: "issue_0001",
            enabled: true,
            policy_revision: 4,
            source: command.source,
            options: command.options,
            target: command.target,
            prepare_intent_id: "prepare_intent_0001",
            plan_id: null,
            session_id: null,
            created_at: "2026-10-16T00:00:00Z",
            updated_at: "2026-10-20T00:00:00Z",
          });
        }
        return jsonResponse(enrollment);
      }
      if (url.includes("/work-item-plans:prepare")) {
        return jsonResponse({
          workspace_session: { workspace_session_id: "ws_0001" },
        });
      }
      throw new Error(`unexpected fetch: ${url}`);
    }),
  );
  return captured;
}

const AUTOMATIC_OPTIONS: WorkItemPlanOptionsFormValue = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
  automation_mode: "automatic",
};

type GenerationProps = {
  selectionKey: string;
};

function renderGeneration(selectionKey: string): {
  actions: () => IssueLifecycleGenerationActions;
  reopen: (nextSelectionKey: string) => void;
} {
  const { result, rerender } = renderHook(
    (props: GenerationProps) =>
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
      }),
    { initialProps: { selectionKey } },
  );
  return {
    // 每次渲染都是新的闭包（props 漂移后必须取最新一份）。
    actions: () => result.current,
    reopen: (nextSelectionKey: string) =>
      rerender({ selectionKey: nextSelectionKey }),
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useIssueLifecycleGeneration：terminated enrollment 上的人工重新开启", () => {
  it("submits an enable command carrying a stable command_id, linearized on the terminated revision", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    // 红测锚点：terminated 态不阻断入口——enable 命令照发。
    expect(captured.enableBodies).toHaveLength(1);
    const body = captured.enableBodies[0];
    expect(body.command.type).toBe("enable");
    // 契约①：Enable 必须带稳定 command_id（缺它后端 422）。
    expect(typeof body.command.command_id).toBe("string");
    expect(body.command.command_id.length).toBeGreaterThan(0);
    // 契约③：expected_revision 取终止记录当前 revision（线性化延续）。
    expect(body.expected_revision).toBe(3);
    // 重新开启是新命令：命令身份不得复用被终止记录那次 Enable 的身份，也
    // 不沿用本次弹窗的 selection_key（重新开启须能被账本单独留痕）。
    expect(body.command.selection_key).not.toBe("selection_old");
    expect(body.command.selection_key).not.toBe(DIALOG_SELECTION_KEY);
  });

  it("echoes the terminated record's authorization payload instead of rewriting it", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    const command = captured.enableBodies[0].command;
    // 契约②：payload 与终止记录逐字段相等——服务端逐字段比对，改了即 409。
    expect(command.source).toEqual({
      stories: [{ id: "story_0001", version: 2 }],
      designs: [{ id: "design_0001", version: 1 }],
    });
    expect(command.options).toEqual({
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
    });
    expect(command.target).toEqual({
      kind: "single_repository",
      repository_id: "00000000-0000-0000-0000-000000000001",
    });
  });

  it("reuses the same command_id when the same re-enable is retried in the same dialog", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);
    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    expect(captured.enableBodies).toHaveLength(2);
    // 同一弹窗实例内重试＝同一次用户动作：command_id 与命令身份都稳定
    // （服务端据此幂等重放首次结果，不重复取代）。
    expect(typeof captured.enableBodies[0].command.command_id).toBe("string");
    expect(captured.enableBodies[1].command.command_id).toBe(
      captured.enableBodies[0].command.command_id,
    );
    expect(captured.enableBodies[1].command.selection_key).toBe(
      captured.enableBodies[0].command.selection_key,
    );
  });

  it("mints a new command_id when the dialog is reopened", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions, reopen } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);
    reopen("selection_0002");
    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    expect(captured.enableBodies).toHaveLength(2);
    // 弹窗重开＝另一次用户动作：command_id 与命令身份都换新。
    expect(captured.enableBodies[1].command.command_id).not.toBe(
      captured.enableBodies[0].command.command_id,
    );
    expect(captured.enableBodies[1].command.selection_key).not.toBe(
      captured.enableBodies[0].command.selection_key,
    );
  });

  it("surfaces the cleanup-required conflict with its own next step", async () => {
    stubLifecycleFetch(terminatedEnrollment(), {
      code: "automation_plan_cleanup_required",
      message: "automation plan cleanup required",
      status: 409,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    // 契约④：收口未完成不是通用冲突——提示先重试终止完成清理。
    await expect(
      actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS),
    ).rejects.toThrow(/先重试终止完成清理/);
  });

  it("keeps the ordinary concurrency semantics for a non-terminated enrollment", async () => {
    const captured = stubLifecycleFetch({
      ...terminatedEnrollment(),
      enabled: true,
      terminated_at: null,
      policy_revision: 2,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    expect(captured.enableBodies).toHaveLength(1);
    const body = captured.enableBodies[0];
    expect(body.expected_revision).toBe(2);
    // 非 terminated：命令身份照旧用弹窗实例的 selection_key。
    expect(body.command.selection_key).toBe(DIALOG_SELECTION_KEY);
    // 常规启用仍按当前已确认的 Story/Design 引用提交 payload。
    expect(body.command.source).toEqual({
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
    });
    expect(typeof body.command.command_id).toBe("string");
  });
});
