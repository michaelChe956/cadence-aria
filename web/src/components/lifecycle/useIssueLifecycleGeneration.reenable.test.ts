import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "../../api/client";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import type { WorkItemPlanOptionsFormValue } from "./WorkItemPlanOptionsDialog";
import type { IssueLifecycleGenerationActions } from "./useIssueLifecycleGeneration";
import { useIssueLifecycleGeneration } from "./useIssueLifecycleGeneration";

// 批 2 T4（REQ-WIGA-09 修订）＋块1（REQ-WIGA-SWAP-01/02）前端段：terminated
// 的 enrollment 上人工「启用自动化」＝**一次完成**的换 provider 换代：
//   ① 只发一条 Enable（不再「先用旧 provider 重开 → Disable → 再 Enable」），
//      携带稳定 command_id（同一动作重试复用同一 id；弹窗重开或换参数换新
//      id）——缺它后端 422；
//   ② source 取**当前 card** 的已确认引用（与常规启用同口径），target 原样
//      回传终止记录；options＝终止记录授权 + 用户本次选择的 provider（其余
//      授权字段逐字保留，差异按换代路径落盘）——旧 source 悬空时服务端放行
//      source 漂移（悬空重授权），仍有效但不同则 409；
//   ③ expected_revision 取终止记录当前 revision（线性化延续）；
//   ④ 失败面按稳定码分诊：运行中改 payload／版本过期各有中文主文案与下一步，
//      旧冲突码仍提示「换代/重绑（rebind）」，收口未完成提示先重试清理；
//   ⑤ 换代失败后重读 enrollment：已非 terminated ⇒ 旧工作项组已删、新未建，
//      如实告知并给重试指引（MUST NOT 说「未发生任何变化」）；
//   ⑥ 非 terminated 的常规并发语义不变。

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

// 新投影只出逻辑代码库两变体（REQ-ROUTE-C5-TARGET 后端侧已下线单仓载体）。
const LOGICAL_TARGET = {
  kind: "logical_codebase",
  logical_codebase_id: "11111111-1111-1111-1111-111111111111",
  logical_repository_id: "00000000-0000-0000-0000-000000000001",
} as const;

/** legacy 只读载体：老单仓 enrollment 记录仍带这个 target（存量数据不丢）。 */
const LEGACY_SINGLE_REPOSITORY_TARGET = {
  kind: "single_repository",
  repository_id: "00000000-0000-0000-0000-000000000001",
} as const;

// 镜像真实后端 GET automation-target 投影 wire。
function automationTargetResponse(url: string): unknown {
  const query = new URL(url, "http://localhost").searchParams;
  return {
    enrollment_target: LOGICAL_TARGET,
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
function terminatedEnrollment(
  target: unknown = LOGICAL_TARGET,
): Record<string, unknown> {
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
    target,
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
 * **terminated 记录的 Enable 只要求 source/target 与终止记录逐字段相等**
 * （REQ-WIGA-SWAP-01：options 含 provider 允许不同，按换代语义落盘），唯一
 * 例外是 `sourceDangling: true`（终止记录的原 source 引用实体已删）时仅放行
 * source 漂移（悬空重授权）；enabled 记录的异 payload Enable 409；disabled
 * 记录换 payload 重开放行；Disable 线性化 +1。`failEnableCount/failEnable`
 * 注入授权写命令失败（零副作用，用于失败面分诊）。
 */
function stubStatefulEnrollmentFetch(
  initial: Record<string, unknown>,
  options?: {
    failDisableCount?: number;
    sourceDangling?: boolean;
    failEnableCount?: number;
    failEnable?: { code: string; message: string; status: number };
  },
): StatefulEnrollmentStub {
  const puts: PutBody[] = [];
  let state: Record<string, unknown> = { ...initial };
  let remainingDisableFailures = options?.failDisableCount ?? 0;
  let remainingEnableFailures = options?.failEnableCount ?? 0;

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
      const sourceTargetMatches =
        JSON.stringify(command.source) === JSON.stringify(state.source) &&
        JSON.stringify(command.target ?? null) ===
          JSON.stringify(state.target ?? null);
      const sourceDriftOnly =
        JSON.stringify(command.source) !== JSON.stringify(state.source) &&
        JSON.stringify(command.target ?? null) ===
          JSON.stringify(state.target ?? null);
      if (terminated && !sourceTargetMatches) {
        // terminated 重开：source/target 逐字段相等（options 允许不同——换代
        // 语义）；悬空源重授权仅放行 source 漂移。
        if (!(options?.sourceDangling && sourceDriftOnly)) {
          return conflict();
        }
      }
      if (remainingEnableFailures > 0) {
        remainingEnableFailures -= 1;
        const failure = options?.failEnable;
        if (!failure) {
          throw new Error("failEnableCount requires failEnable");
        }
        return jsonResponse(
          { code: failure.code, message: failure.message, details: {} },
          failure.status,
        );
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

  it("submits the current card source and the terminated target, applying the provider the user picked", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS);

    expect(captured.enableBodies).toHaveLength(1);
    const command = captured.enableBodies[0].command;
    // 契约②：source 取当前 card 的已确认引用（story v3/design v2，与常规
    // 启用同口径）；target 原样回传终止记录。
    expect(command.source).toEqual({
      stories: [{ id: "story_0001", version: 3 }],
      designs: [{ id: "design_0001", version: 2 }],
    });
    expect(command.target).toEqual({
      kind: "logical_codebase",
      logical_codebase_id: "11111111-1111-1111-1111-111111111111",
      logical_repository_id: "00000000-0000-0000-0000-000000000001",
    });
    // options 不再原样 echo 旧值：＝终止记录的授权 + 用户本次选择的 provider
    // （其余授权字段逐字保留，不被表单里的未生效值改写）。
    expect(command.options).toEqual({
      ...(terminatedEnrollment().options as Record<string, unknown>),
      author_provider: "codex",
      reviewer_provider: "pi",
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

  it("applies the provider the user picked in a single re-enable command, with no disable/enable churn", async () => {
    const stub = stubStatefulEnrollmentFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS);

    // REQ-WIGA-SWAP-01：一次提交＝一条命令——Enable 直接带用户选的新
    // provider（不再先用旧 provider 重开，也不再 Disable→Enable）。
    expect(stub.puts.map((put) => put.command.type)).toEqual(["enable"]);
    expect(stub.puts).toHaveLength(1);
    // 线性化延续：expected_revision 取终止记录当前 revision。
    expect(stub.puts[0].expected_revision).toBe(3);
    // 请求体的 provider 已是新值。
    expect(stub.puts[0].command.options).toMatchObject({
      author_provider: "codex",
      reviewer_provider: "pi",
    });
    // source/target 仍与终止记录逐字段相等（换代只换 options）。
    expect(stub.puts[0].command.source).toEqual(
      terminatedEnrollment().source,
    );
    expect(stub.puts[0].command.target).toEqual(
      terminatedEnrollment().target,
    );
    const final = stub.current();
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

  it("issues exactly one enable and no disable/enable churn when the provider is unchanged", async () => {
    const stub = stubStatefulEnrollmentFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    // 未换 provider：同样只发一条 Enable，options 与终止记录一致。
    expect(stub.puts.map((put) => put.command.type)).toEqual(["enable"]);
    expect(stub.puts[0].expected_revision).toBe(3);
    expect(stub.puts[0].command.options).toEqual(terminatedEnrollment().options);
    const final = stub.current();
    expect(final.enabled).toBe(true);
    expect(final.terminated_at).toBeNull();
  });

  it("does not replay a re-enable sequence after a failed step, and reports the stale revision", async () => {
    const stub = stubStatefulEnrollmentFetch(terminatedEnrollment(), {
      failEnableCount: 1,
      failEnable: {
        code: "automation_enrollment_revision_stale",
        message: "automation enrollment revision is stale",
        status: 409,
      },
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    // 第一次提交失败（版本过期，服务端零副作用）：只有一条 Enable，没有
    // Disable 也没有序列重放；失败面说明「本次操作没有生效」并指向重试。
    await expect(
      actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS),
    ).rejects.toThrow(/平台的授权记录刚发生变化，本次操作没有生效/);
    expect(stub.puts.map((put) => put.command.type)).toEqual(["enable"]);
    // 不留半开状态：记录仍是终止态，revision 与授权都没有被改写。
    expect(stub.current().terminated_at).not.toBeNull();
    expect(stub.current().policy_revision).toBe(3);
    expect(stub.current().options).toEqual(terminatedEnrollment().options);

    // 重试：仍是一条 Enable，直接把换代补齐。
    await actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS);
    expect(stub.puts.map((put) => put.command.type)).toEqual([
      "enable",
      "enable",
    ]);
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

  it("explains that a running automation cannot swap providers and points at the termination entry", async () => {
    // REQ-WIGA-SWAP-02：运行态限制是专属稳定码——中文主文案说明「运行中不能
    // 直接改」并指明下一步（在既有确认面终止整个自动化后回来重新启用）。
    stubLifecycleFetch(terminatedEnrollment(), {
      code: "automation_enrollment_payload_immutable",
      message: "automation enrollment payload is immutable while running",
      status: 409,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    const failure = await actions()
      .handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS)
      .catch((reason: unknown) => reason);

    expect(failure).toBeInstanceOf(Error);
    const message = (failure as Error).message;
    expect(message).toContain("这个自动化正在运行中，不能直接换 provider");
    expect(message).toContain("终止整个自动化");
    // 机器码不进主文案（只作次要信息）。
    expect(message).not.toContain("automation_enrollment_payload_immutable");
  });

  it("tells the user the authorization just changed and to retry once the run stops", async () => {
    stubLifecycleFetch(terminatedEnrollment(), {
      code: "automation_enrollment_revision_stale",
      message: "automation enrollment revision is stale",
      status: 409,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    const failure = await actions()
      .handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS)
      .catch((reason: unknown) => reason);

    expect(failure).toBeInstanceOf(Error);
    const message = (failure as Error).message;
    expect(message).toContain("平台的授权记录刚发生变化，本次操作没有生效");
    expect(message).toContain("等当前生成停下来再点一次");
    expect(message).not.toContain("automation_enrollment_revision_stale");
  });

  it("keeps the rebind hint for the legacy conflict code", async () => {
    // 回归锁：其余 7 类冲突仍落回旧码，文案不得因稳定码细分而丢失下一步。
    stubLifecycleFetch(terminatedEnrollment(), {
      code: "automation_enrollment_conflict",
      message: "automation enrollment revision conflict",
      status: 409,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await expect(
      actions().handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS),
    ).rejects.toThrow(/换代 \/ 重新绑定（rebind）/);
  });

  it("tells the user the old work items are gone but the new ones were not built", async () => {
    // REQ-WIGA-SWAP-01 失败面：换代在「已删除旧工作项组」之后失败 —— 重读
    // enrollment 显示 terminated_at 已清空（取代已落盘），必须如实告知
    // 「已删未建」并给重试指引，MUST NOT 说「未发生任何变化」。
    const puts: PutBody[] = [];
    let gets = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (!url.includes("/automation-enrollment")) {
          throw new Error(`unexpected fetch: ${url}`);
        }
        if (init?.method === "PUT") {
          puts.push(JSON.parse(String(init.body)) as PutBody);
          return jsonResponse(
            {
              code: "automation_enrollment_rebuild_failed",
              message: "rebuild failed after the previous work items were deleted",
              details: {},
            },
            500,
          );
        }
        gets += 1;
        // 第一次读：终止态；失败后重读：旧工作项组已删（terminated_at 清空）。
        return jsonResponse(
          gets === 1
            ? terminatedEnrollment()
            : {
                ...terminatedEnrollment(),
                enabled: true,
                terminated_at: null,
                policy_revision: 4,
              },
        );
      }),
    );
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    const failure = await actions()
      .handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS)
      .catch((reason: unknown) => reason);

    expect(puts).toHaveLength(1);
    expect(failure).toBeInstanceOf(Error);
    const message = (failure as Error).message;
    expect(message).toContain("上一次的工作已经被清空");
    expect(message).toContain("新的没建起来");
    expect(message).toContain("重试");
    expect(message).not.toContain("未发生任何变化");
  });

  it("keeps the machine code on the wrapped re-enable failure for the secondary line", async () => {
    // REQ-WIGA-SWAP-02：机器码只是「降级」为次要信息，MUST NOT 被包装丢掉——
    // 中文主文案 + 原始 code 同时带回，弹窗据此把码渲染成次要行、把中文渲染
    // 成主文案。
    stubLifecycleFetch(terminatedEnrollment(), {
      code: "automation_enrollment_revision_stale",
      message: "automation enrollment revision is stale",
      status: 409,
    });
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    const failure = await actions()
      .handleConfirmWorkItemPlanOptions(CHOSEN_PROVIDER_OPTIONS)
      .catch((reason: unknown) => reason);

    expect(failure).toBeInstanceOf(ApiRequestError);
    // 主文案是中文成品文案。
    expect((failure as Error).message).toContain(
      "平台的授权记录刚发生变化，本次操作没有生效",
    );
    // 机器码与原始 details 仍在（弹窗降级为次要行 / 折叠区）。
    expect((failure as ApiRequestError).code).toBe(
      "automation_enrollment_revision_stale",
    );
    expect((failure as ApiRequestError).details).toEqual({});
  });
});

// 发起侧载体收窄（方案 A，REQ-ROUTE-C5-TARGET 前端侧）：**命令侧**只接受
// 逻辑代码库两变体。legacy 单仓载体（仅 legacy 只读投影）⇒ 守卫返回 null
// ⇒ fail-closed：**不发命令**、不静默降级、不改绑 LC。
describe("useIssueLifecycleGeneration：命令侧载体收窄", () => {
  it("refuses to re-enable a legacy single-repository terminated record", async () => {
    const captured = stubLifecycleFetch(
      terminatedEnrollment(LEGACY_SINGLE_REPOSITORY_TARGET),
    );
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await expect(
      actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS),
    ).rejects.toThrow(/需重建为逻辑代码库授权/);
    // 零副作用：不发 Enable 命令。
    expect(captured.enableBodies).toHaveLength(0);
  });

  it("refuses to enable when the server projects a legacy single-repository carrier", async () => {
    const puts: PutBody[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes("/automation-target")) {
          const projection = automationTargetResponse(url) as Record<
            string,
            unknown
          >;
          return jsonResponse({
            ...projection,
            enrollment_target: LEGACY_SINGLE_REPOSITORY_TARGET,
          });
        }
        if (url.includes("/automation-enrollment")) {
          if (init?.method === "PUT") {
            puts.push(JSON.parse(String(init.body)) as PutBody);
            return jsonResponse({});
          }
          return jsonResponse(null);
        }
        throw new Error(`unexpected fetch: ${url}`);
      }),
    );
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await expect(
      actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS),
    ).rejects.toThrow(/需重建为逻辑代码库授权/);
    // 零副作用：不发 Enable 命令。
    expect(puts).toHaveLength(0);
  });

  it("never puts a single_repository carrier on the wire", async () => {
    const captured = stubLifecycleFetch(terminatedEnrollment());
    const { actions } = renderGeneration(DIALOG_SELECTION_KEY);

    await actions().handleConfirmWorkItemPlanOptions(AUTOMATIC_OPTIONS);

    // 命令侧只出逻辑代码库两变体——线上载荷里不得出现单仓载体。
    expect(captured.enableBodies).toHaveLength(1);
    const target = captured.enableBodies[0].command.target as {
      kind?: string;
    };
    expect(target.kind).not.toBe("single_repository");
    expect(target.kind).toBe("logical_codebase");
  });
});
