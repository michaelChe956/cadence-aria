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
//   ② 重新开启的 source 取**当前 card** 的已确认引用（与常规启用同口径）；
//      options/target 原样回传终止记录（服务端比对）——旧 source 悬空时
//      服务端放行 source 漂移（悬空重授权），仍有效但不同则 409；
//   ③ expected_revision 取终止记录当前 revision（线性化延续）；
//   ④ 收口未完成（409 automation_plan_cleanup_required）提示先重试清理；
//      重新开启 409 冲突提示走「换代/重绑（rebind）」可行动下一步；
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

/**
 * 已终止的 enrollment 投影（terminated_at 非空即终态事实）。source 与当前
 * card 一致（情形②：旧 source 仍有效且相同 → 正常等 payload 重新开启）。
 */
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

/** 情形①：终止记录的 source 指向已删除实体（悬空）——服务端允许按当前
 *  已确认 source 重新授权（resign）。 */
function danglingSourceTerminatedEnrollment(): Record<string, unknown> {
  return {
    ...terminatedEnrollment(),
    source: {
      stories: [{ id: "story_spec_deleted", version: 2 }],
      designs: [{ id: "design_spec_deleted", version: 1 }],
    },
  };
}

/** 情形③：终止记录的 source 引用的实体仍在（story_0001/design_0001 存在）
 *  但版本与当前不同——非悬空，服务端维持逐字段相等 409。 */
function divergentSourceTerminatedEnrollment(): Record<string, unknown> {
  return {
    ...terminatedEnrollment(),
    source: {
      stories: [{ id: "story_0001", version: 2 }],
      designs: [{ id: "design_0001", version: 1 }],
    },
  };
}

/** 镜像服务端 `validate_confirmed_source_refs`：提交的 source 必须 id 存在、
 *  已确认且版本与 current_version 精确一致（悬空/过期引用一律 422）。 */
function sourceMatchesCurrentCards(source: unknown): boolean {
  const value = source as {
    stories?: { id: string; version: number }[];
    designs?: { id: string; version: number }[];
  };
  const refsOk = (
    refs: { id: string; version: number }[] | undefined,
    cards: LifecycleCard[],
    specKind: "story_spec" | "design_spec",
  ) =>
    (refs?.length ?? 0) > 0 &&
    refs!.every((ref) =>
      cards.some(
        (card) =>
          card.kind === specKind &&
          card.id === ref.id &&
          card.raw.confirmation_status === "confirmed" &&
          (card.raw.current_version ?? 1) === ref.version,
      ),
    );
  return (
    refsOk(value.stories, COLUMNS.story_spec, "story_spec") &&
    refsOk(value.designs, COLUMNS.design_spec, "design_spec")
  );
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

/** 用户本次选择的 provider（重新开启时换 provider 的场景）。 */
const CHOSEN_PROVIDER_OPTIONS: WorkItemPlanOptionsFormValue = {
  ...AUTOMATIC_OPTIONS,
  author_provider: "codex",
  reviewer_provider: "pi",
};

type PutBody = {
  expected_revision: number | null;
  command: {
    type: string;
    command_id?: string;
    selection_key?: string;
    source?: Record<string, unknown>;
    options?: Record<string, unknown>;
    target?: Record<string, unknown>;
  };
};

type StatefulEnrollmentStub = {
  puts: PutBody[];
  current: () => Record<string, unknown>;
};

function samePayload(
  command: PutBody["command"],
  source: unknown,
  options: unknown,
  target: unknown,
): boolean {
  return (
    JSON.stringify(command.source) === JSON.stringify(source) &&
    JSON.stringify(command.options) === JSON.stringify(options) &&
    JSON.stringify(command.target ?? null) === JSON.stringify(target ?? null)
  );
}

/**
 * 有状态的 enrollment 夹具：镜像服务端语义——提交 source 先过
 * `validate_confirmed_source_refs`（id 存在/已确认/版本精确一致，否则 422）；
 * terminated 记录的 Enable 必须与终止记录逐字段相等（否则 409），唯一例外
 * 是 `sourceDangling: true`（终止记录的原 source 引用实体已删）时仅放行
 * source 漂移（悬空重授权）；enabled 记录的异 payload Enable 409；disabled
 * 记录换 payload 重开放行；Disable 线性化 +1。
 */
function stubStatefulEnrollmentFetch(
  initial: Record<string, unknown>,
  options?: { failDisableCount?: number; sourceDangling?: boolean },
): StatefulEnrollmentStub {
  const puts: PutBody[] = [];
  let state: Record<string, unknown> = { ...initial };
  let remainingDisableFailures = options?.failDisableCount ?? 0;

  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/automation-target")) {
        return jsonResponse(automationTargetResponse(url));
      }
      if (!url.includes("/automation-enrollment")) {
        throw new Error(`unexpected fetch: ${url}`);
      }
      if (init?.method !== "PUT") {
        return jsonResponse(state);
      }
      const body = JSON.parse(String(init.body)) as PutBody;
      puts.push(body);
      const revision = Number(state.policy_revision ?? 0);
      const conflict = () =>
        jsonResponse(
          {
            code: "automation_enrollment_conflict",
            message: "automation enrollment revision conflict",
            details: {},
          },
          409,
        );
      if (body.expected_revision !== revision) {
        return conflict();
      }
      const command = body.command;
      if (command.type === "disable") {
        if (remainingDisableFailures > 0) {
          remainingDisableFailures -= 1;
          return conflict();
        }
        state = {
          ...state,
          enabled: false,
          policy_revision: revision + 1,
        };
        return jsonResponse(state);
      }
      // 提交 source 的三条判定不放宽：悬空/过期引用 → 422 零副作用。
      if (!sourceMatchesCurrentCards(command.source)) {
        return jsonResponse(
          {
            code: "automation_enrollment_invalid_scope",
            message: "story spec not found or stale for automation enrollment",
            details: {},
          },
          422,
        );
      }
      const terminated = state.terminated_at != null;
      const payloadMatches = samePayload(
        command,
        state.source,
        state.options,
        state.target,
      );
      const sourceDriftOnly =
        JSON.stringify(command.source) !== JSON.stringify(state.source) &&
        JSON.stringify(command.options) === JSON.stringify(state.options) &&
        JSON.stringify(command.target ?? null) ===
          JSON.stringify(state.target ?? null);
      if (terminated && !payloadMatches) {
        // 重新开启与终止记录逐字段相等；悬空源重授权仅放行 source 漂移。
        if (!(options?.sourceDangling && sourceDriftOnly)) {
          return conflict();
        }
      }
      if (!terminated && state.enabled) {
        if (!payloadMatches) {
          return conflict();
        }
        // 同 payload 幂等重放：revision 不递增。
        return jsonResponse(state);
      }
      state = {
        ...state,
        enabled: true,
        terminated_at: null,
        policy_revision: revision + 1,
        selection_key: command.selection_key,
        source: command.source,
        options: command.options,
        target: command.target,
        prepare_intent_id: "prepare_intent_0002",
        plan_id: null,
        session_id: null,
      };
      return jsonResponse(state);
    }),
  );
  return { puts, current: () => state };
}

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

  it("submits the current card source while echoing the terminated record's options and target", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    const command = captured.enableBodies[0].command;
    // 契约②：source 取当前 card 的已确认引用（story v3/design v2，与常规
    // 启用同口径）；options/target 原样回传终止记录（授权不被顺便改写）。
    expect(command.source).toEqual({
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
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

  it("applies the provider the user picked: re-enable with the terminated payload, then disable, then re-enable with the new payload", async () => {
    const stub = stubStatefulEnrollmentFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS);

    // 契约：改授权 payload 须先成功重新开启（等 payload），再走既有
    // Disable→Enable 路径——三步命令，顺序固定。
    expect(stub.puts.map((put) => put.command.type)).toEqual([
      "enable",
      "disable",
      "enable",
    ]);
    // ① 重新开启：payload 与终止记录逐字段相等（不顺便改写授权）。
    expect(stub.puts[0].command.options).toEqual(terminatedEnrollment().options);
    expect(stub.puts[0].command.source).toEqual(
      terminatedEnrollment().source,
    );
    // ② Disable 线性化在重新开启之后。
    expect(stub.puts[1].expected_revision).toBe(4);
    // ③ 换 provider 后重新开启：终态 enrollment 的 options 是用户选的 provider。
    const final = stub.current();
    expect(final.options).toMatchObject({
      author_provider: "codex",
      reviewer_provider: "pi",
    });
    expect(final.enabled).toBe(true);
    expect(final.terminated_at).toBeNull();
    // 其余授权字段不被表单里的未生效值改写。
    const terminatedOptions = terminatedEnrollment().options as Record<
      string,
      unknown
    >;
    expect(final.options).toEqual({
      ...terminatedOptions,
      author_provider: "codex",
      reviewer_provider: "pi",
    });
  });

  it("keeps the terminated payload and issues no disable/enable churn when the provider is unchanged", async () => {
    const stub = stubStatefulEnrollmentFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    // 未换 provider：只发一条等 payload 的重新开启，不产生多余序列。
    expect(stub.puts).toHaveLength(1);
    expect(stub.puts[0].command.type).toBe("enable");
    expect(stub.puts[0].command.options).toEqual(terminatedEnrollment().options);
    const revisionAfterReenable = stub.current().policy_revision;

    // 幂等重试：同一动作重放不递增 revision、不发新命令。
    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);
    expect(stub.puts).toHaveLength(1);
    expect(stub.current().policy_revision).toBe(revisionAfterReenable);
  });

  it("resumes the re-enable sequence after a failed step instead of leaving a half-open enrollment", async () => {
    const stub = stubStatefulEnrollmentFetch(terminatedEnrollment(), {
      failDisableCount: 1,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    // 第一次提交：重新开启成功，Disable 失败 → 提交报错（授权尚未换 provider）。
    await expect(
      actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS),
    ).rejects.toThrow(/启用自动化失败|revision conflict/);
    expect(stub.current().options).toMatchObject({
      author_provider: "claude_code",
      reviewer_provider: "codex",
    });

    // 重试：从服务端现状续跑（不重复取代），把换 provider 补齐。
    await actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS);
    expect(stub.current().options).toMatchObject({
      author_provider: "codex",
      reviewer_provider: "pi",
    });
    expect(stub.current().enabled).toBe(true);
  });

  it("resigns on the current card source when the terminated record's source is dangling", async () => {
    // 情形①：终止记录的 source 引用已删实体（story_spec_deleted 等）——
    // 重新开启提交**当前 card** 的已确认 source（v3/v2），服务端按悬空
    // 重授权放行；未换 provider 不产生 Disable/Enable churn。
    const stub = stubStatefulEnrollmentFetch(danglingSourceTerminatedEnrollment(), {
      sourceDangling: true,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    expect(stub.puts).toHaveLength(1);
    const command = stub.puts[0].command;
    expect(command.type).toBe("enable");
    expect(command.source).toEqual({
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
    });
    // options/target 仍回传终止记录（悬空重授权只换 source）。
    expect(command.options).toEqual(danglingSourceTerminatedEnrollment().options);
    expect(command.target).toEqual(danglingSourceTerminatedEnrollment().target);
    const final = stub.current();
    expect(final.enabled).toBe(true);
    expect(final.terminated_at).toBeNull();
    expect(final.source).toEqual({
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
    });
  });

  it("surfaces an actionable rebind hint when the original source is valid but divergent", async () => {
    // 情形③：终止记录的 source 引用的实体仍在（story_0001 v2，当前 v3）——
    // 非悬空：按当前 source 提交被服务端 409，提示走「换代/重绑（rebind）」。
    const stub = stubStatefulEnrollmentFetch(divergentSourceTerminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await expect(
      actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS),
    ).rejects.toThrow(/换代|重绑|rebind/i);
    expect(stub.puts).toHaveLength(1);
    expect(stub.puts[0].command.source).toEqual({
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
    });
    // 409 零副作用：enrollment 保持终止态、revision 不变。
    expect(stub.current().terminated_at).not.toBeNull();
    expect(stub.current().policy_revision).toBe(3);
  });
});
