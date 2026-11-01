import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import type { WorkItemPlanOptionsFormValue } from "./WorkItemPlanOptionsDialog";
import type { IssueLifecycleGenerationActions } from "./useIssueLifecycleGeneration";
import { useIssueLifecycleGeneration } from "./useIssueLifecycleGeneration";

// B2-P1 前端段（REQ-WIGA-02）：自动化入口的 Enable 命令显式提交
// `plan_approval: "auto"`——GET automation-target 投影与 prepare 同源解析、
// `plan_approval` 恒为缺省 manual，冻结位由入口选择覆盖；手动链路不经
// enrollment（prepare payload 不携带该字段），弹窗「子 Work Item 执行前
// 需要确认 Plan」复选框行为不变（require_execution_plan_confirm 仍原样
// 透传查询与投影，压 false 归后端冻结归一，前端不改）。

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

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// 镜像真实后端 GET 投影 wire（`automation_target.rs` 同源解析）：
// `plan_approval` 恒 manual、复选项按 query 透传解析。
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
    selection_key: string;
    source: Record<string, unknown>;
    options: Record<string, unknown>;
    target?: Record<string, unknown>;
  };
};

type CapturedRequests = {
  enableBodies: EnableCommandBody[];
  prepareBodies: Record<string, unknown>[];
  targetGets: string[];
};

function stubLifecycleFetch(): CapturedRequests {
  const captured: CapturedRequests = {
    enableBodies: [],
    prepareBodies: [],
    targetGets: [],
  };
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
          const command = body.command;
          return jsonResponse({
            enrollment_id: "enrollment_0001",
            selection_key: command.selection_key,
            project_id: "project_0001",
            issue_id: "issue_0001",
            enabled: true,
            policy_revision: 1,
            source: command.source,
            options: command.options,
            target: command.target,
            prepare_intent_id: "prepare_intent_0001",
            plan_id: null,
            session_id: null,
            created_at: "2026-10-16T00:00:00Z",
            updated_at: "2026-10-16T00:00:00Z",
          });
        }
        return jsonResponse(null);
      }
      if (url.includes("/work-item-plans:prepare")) {
        captured.prepareBodies.push(
          JSON.parse(String(init?.body ?? "{}")) as Record<string, unknown>,
        );
        return jsonResponse({
          workspace_session: { workspace_session_id: "ws_0001" },
        });
      }
      throw new Error(`unexpected fetch: ${url}`);
    }),
  );
  return captured;
}

function renderGeneration(
  options: WorkItemPlanOptionsFormValue,
): IssueLifecycleGenerationActions {
  const { result } = renderHook(() =>
    useIssueLifecycleGeneration({
      selectedProjectId: "project_0001",
      selectedColumns: COLUMNS,
      pendingWorkItemPlanLaunch: {
        card: DESIGN_CARD,
        options,
        selectionKey: "selection_0001",
      },
      setPendingWorkItemPlanLaunch: vi.fn(),
      setError: vi.fn(),
      setSelectedCardKey: vi.fn(),
      refresh: vi.fn(async () => undefined),
      openDrawer: vi.fn(),
      onOpenWorkspace: vi.fn(),
    }),
  );
  return result.current;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useIssueLifecycleGeneration plan_approval（B2-P1 前端段）", () => {
  it("submits plan_approval auto on the Enable command, keeping the server-resolved options otherwise untouched", async () => {
    const captured = stubLifecycleFetch();
    const actions = renderGeneration({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
      automation_mode: "automatic",
    });

    await actions.handleConfirmWorkItemPlanOptions({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
      automation_mode: "automatic",
    });

    expect(captured.enableBodies).toHaveLength(1);
    const options = captured.enableBodies[0].command.options;
    // 红测锚点：GET 投影 resolved_options 恒 manual，入口必须覆盖为 auto。
    expect(options.plan_approval).toBe("auto");
    // 其余字段仍取服务端投影原样（前端不猜默认、不丢字段）。
    expect(options).toEqual({
      author_provider: "codex",
      reviewer_provider: "claude_code",
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      plan_approval: "auto",
      plan_options: {
        include_integration_tests: true,
        include_e2e_tests: false,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
      },
    });
  });

  it("keeps the manual entry on prepare: no enrollment PUT, no plan_approval key, checkbox value unchanged", async () => {
    const captured = stubLifecycleFetch();
    const actions = renderGeneration({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: true,
      automation_mode: "manual",
    });

    await actions.handleConfirmWorkItemPlanOptions({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: true,
      automation_mode: "manual",
    });

    expect(captured.enableBodies).toHaveLength(0);
    expect(captured.prepareBodies).toHaveLength(1);
    const prepare = captured.prepareBodies[0];
    expect(prepare.require_execution_plan_confirm).toBe(true);
    expect("plan_approval" in prepare).toBe(false);
    expect("automation_mode" in prepare).toBe(false);
  });

  it("pins require_execution_plan_confirm=false on the Enable wire in automatic mode", async () => {
    const captured = stubLifecycleFetch();
    const actions = renderGeneration({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: true,
      automation_mode: "automatic",
    });

    await actions.handleConfirmWorkItemPlanOptions({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: true,
      automation_mode: "automatic",
    });

    // 表单值仍参与 target 投影解析（弹窗不篡改表单值；复选框在
    // automatic 下由弹窗置灰固定关闭，此处兜底旧表单值/投影解析值）。
    expect(captured.targetGets).toHaveLength(1);
    expect(captured.targetGets[0]).toContain("require_execution_plan_confirm=true");
    // F2 修复：Enable wire 与后端冻结值一致——auto 一律显式 false
    // （`EnrollmentOptions::normalized_for_enrollment_freeze` 同值），
    // 不再依赖服务端归一，投影解析的 true 不上 wire。
    expect(captured.enableBodies).toHaveLength(1);
    const options = captured.enableBodies[0].command.options;
    expect(options.plan_approval).toBe("auto");
    expect(options.plan_options).toMatchObject({
      require_execution_plan_confirm: false,
    });
  });
});
