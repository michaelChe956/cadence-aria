// Task 9.5：知识库卡片的宿主接线测试。
// 覆盖四件事：
//   1. 抽屉展开后卡片拿到 status（GET .../knowledge-base/status）与 active
//      operation（GET .../knowledge-base/initializations/active）——不再空态；
//   2. 点「知识库初始化」发起 POST .../knowledge-base/initializations，
//      body 带 kind=bootstrap 与每次唯一的幂等键；
//   3. 点「知识库更新」把选中的变更包路径透传进 POST（package_path 不能丢——
//      丢了后端拿不到变更包，门禁形同虚设）；
//   4. 点「查看知识库详细内容」跳 Task 10 的详情页路由。
// 修复轮 1（审查 Important/Minor）另补四条：
//   5. LC 快速切换：旧 LC 的慢响应不覆盖当前 LC 的状态（过期守卫）；
//   6. status 取数 500 → 错误横幅可见、页面不崩；
//   7. 启动接口 4xx → 预检失败进卡片、错误横幅可见、页面不崩；
//   8. 启动接口 200 + 异常载荷 → 不崩、卡片不挂脏 operation（形状守卫）。
// 终审修复（C-2）补一条：
//   9. operation 运行中每 2s 轮询（与聚合初始化卡同构）；到终态停止拉取。
// 终审复审 Minor（M4）补一条：
//  10. 轮询失败静默重试（不刷错误横幅）；用户主动动作（取消）失败仍报错。
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  jsonResponseValue,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

const PROJECT_ID = "project_0001";
const LC_ID = "lc_0001";
const PACKAGE_PATH = "cadence/knowledge-base/changes/add-login-feature";

function knowledgeBaseStatus() {
  return {
    exists: true,
    schema_version: "4.0",
    initialization_status: "complete",
    global_validation: "passed",
    completed_stages: ["scope", "structure"],
    skipped_stages: [],
    skills_available: true,
  };
}

function knowledgeBaseOperation(overrides: Record<string, unknown> = {}) {
  return {
    operation_id: "kb_op_0001",
    project_id: PROJECT_ID,
    logical_codebase_id: LC_ID,
    kind: "bootstrap",
    input: {
      idempotency_key: "idem-0001",
      provider: "kimi",
      package_path: null,
      package_id: null,
    },
    status: "running",
    steps: [
      {
        step_id: "generate",
        status: "running",
        started_at: null,
        completed_at: null,
        output_summary: null,
      },
    ],
    provider_selection: null,
    error: null,
    created_at: "2026-11-09T00:00:00Z",
    started_at: null,
    completed_at: null,
    ...overrides,
  };
}

function knowledgeBasePackage() {
  return {
    id: "pkg_0001",
    path: PACKAGE_PATH,
    violation_count: 0,
    violations: [],
  };
}

/**
 * 既有工作台夹具 + 知识库端点路由。返回发起记录，供断言请求体。
 * options 允许按 LC 覆盖各端点响应（竞态测试需要「旧 LC 慢、新 LC 快」）。
 */
interface KnowledgeBaseFetchOptions {
  /** 每个 LC 的 status 响应（默认统一返回 knowledgeBaseStatus()）。 */
  statusByLc?: Record<string, () => Promise<Response>>;
  /** 每个 LC 的 active operation 响应（默认统一返回 running operation）。 */
  activeByLc?: Record<string, () => Promise<Response>>;
  /** 启动接口响应（默认 200 + running operation）。 */
  startResponse?: () => Promise<Response>;
  /** 工作台里的逻辑代码库列表（默认只有 lc_0001）。 */
  logicalCodebases?: Array<{ id: string; name: string; member_count: number }>;
  /** 取消接口响应（默认 200 + cancelled operation）。 */
  cancelResponse?: () => Promise<Response>;
}

/** 知识库 HTTP 面：/api/projects/{p}/logical-codebases/{lc}/knowledge-base/... */
const KB_PATH_PATTERN =
  /^\/api\/projects\/[^/]+\/logical-codebases\/([^/]+)\/knowledge-base(\/.*)?$/u;

function installKnowledgeBaseFetch(options: KnowledgeBaseFetchOptions = {}) {
  const base = lifecycleFetch({
    projects: [projectRecord(PROJECT_ID, "Aria")],
    logicalCodebases: options.logicalCodebases ?? [
      { id: LC_ID, name: "platform", member_count: 1 },
    ],
  });
  const startBodies: Array<Record<string, unknown>> = [];
  const fetchMock = vi.fn(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      const matched = KB_PATH_PATTERN.exec(url);
      if (matched) {
        const lcId = matched[1] ?? "";
        const rest = matched[2] ?? "";
        if (rest === "/status") {
          return (
            options.statusByLc?.[lcId]?.() ?? jsonResponse(knowledgeBaseStatus())
          );
        }
        if (rest === "/initializations/active") {
          return (
            options.activeByLc?.[lcId]?.() ??
            jsonResponse(knowledgeBaseOperation())
          );
        }
        if (rest === "/change-packages") {
          return jsonResponse({ packages: [knowledgeBasePackage()] });
        }
        if (rest === "/initializations" && method === "POST") {
          startBodies.push(
            JSON.parse(String(init?.body)) as Record<string, unknown>,
          );
          return (
            options.startResponse?.() ??
            jsonResponse(knowledgeBaseOperation({ status: "running" }))
          );
        }
        if (/^\/initializations\/[^/]+\/cancel$/u.test(rest) && method === "POST") {
          return (
            options.cancelResponse?.() ??
            jsonResponse(knowledgeBaseOperation({ status: "cancelled" }))
          );
        }
        if (rest === "/bootstrap-inputs" && method === "POST") {
          return jsonResponse({ created: ["api-scope.md"] });
        }
      }
      return base(input, init);
    },
  );
  vi.stubGlobal("fetch", fetchMock);
  return { fetchMock, startBodies };
}

function renderWorkbench() {
  const rootRoute = createRootRoute({
    component: () => (
      <>
        <IssueLifecycleWorkbench />
        <Outlet />
      </>
    ),
  });
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/",
    component: () => null,
  });
  const knowledgeBaseRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/workbench/projects/$projectId/logical-codebases/$lcId/knowledge-base",
    component: () => <div data-testid="knowledge-base-detail-page">知识库详情</div>,
  });
  const router = createRouter({
    routeTree: rootRoute.addChildren([indexRoute, knowledgeBaseRoute]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

describe("IssueLifecycleWorkbench 知识库卡片宿主接线（Task 9.5）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("抽屉展开后卡片拿到 status 与 active operation（不再是空态）", async () => {
    installKnowledgeBaseFetch();
    const user = userEvent.setup();

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));

    const card = await screen.findByTestId("knowledge-base-card");
    // status：initialization_status=complete ⇒ 「知识库更新」按钮解禁
    // （未拿到 status 时该按钮恒为禁用）。
    await waitFor(() =>
      expect(
        within(card).getByRole("button", { name: "知识库更新" }),
      ).not.toBeDisabled(),
    );
    // operation：active operation 的状态徽标。
    expect(
      within(card).getByTestId("knowledge-base-operation-status"),
    ).toHaveAttribute("data-status", "running");
  });

  it("点「知识库初始化」发起 POST initializations（kind=bootstrap + 唯一幂等键）", async () => {
    const { startBodies } = installKnowledgeBaseFetch();
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(
      await screen.findByRole("button", { name: "知识库初始化" }),
    );

    await waitFor(() => expect(startBodies).toHaveLength(1));
    expect(startBodies[0]).toMatchObject({ kind: "bootstrap" });
    const idempotencyKey = startBodies[0]?.idempotency_key;
    expect(typeof idempotencyKey).toBe("string");
    expect(idempotencyKey).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u,
    );
  });

  it("点「知识库更新」把变更包路径透传进 POST（package_path 不丢）", async () => {
    const { startBodies } = installKnowledgeBaseFetch();
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "知识库更新" }));

    const dialog = await screen.findByTestId("knowledge-base-update-dialog");
    const packageLabel = await within(dialog).findByTestId(
      `knowledge-base-package-${PACKAGE_PATH}`,
    );
    await user.click(within(packageLabel).getByRole("radio"));
    await user.click(within(dialog).getByRole("button", { name: "执行更新" }));

    await waitFor(() => expect(startBodies).toHaveLength(1));
    expect(startBodies[0]).toMatchObject({
      kind: "update",
      package_path: PACKAGE_PATH,
    });
    expect(typeof startBodies[0]?.idempotency_key).toBe("string");
  });

  it("点「查看知识库详细内容」跳知识库详情页路由", async () => {
    installKnowledgeBaseFetch();
    const user = userEvent.setup();

    const router = renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(
      await screen.findByRole("button", { name: "查看知识库详细内容" }),
    );

    await waitFor(() =>
      expect(router.state.location.pathname).toBe(
        `/workbench/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/knowledge-base`,
      ),
    );
    expect(await screen.findByTestId("knowledge-base-detail-page")).toBeInTheDocument();
  });

  it("LC 快速切换：旧 LC 的慢响应不覆盖当前 LC 的状态（过期守卫）", async () => {
    const secondLcId = "lc_0002";
    const aStep = {
      step_id: "a_step",
      status: "running",
      started_at: null,
      completed_at: null,
      output_summary: null,
    };
    const bStep = { ...aStep, step_id: "b_step" };
    // 旧 LC（大 KB）的 status 慢，新 LC 的快——正好构成「旧响应晚到」的时序。
    let releaseSlowStatus!: () => void;
    const slowStatus = new Promise<Response>((resolve) => {
      releaseSlowStatus = () => resolve(jsonResponseValue(knowledgeBaseStatus()));
    });
    installKnowledgeBaseFetch({
      logicalCodebases: [
        { id: LC_ID, name: "platform", member_count: 1 },
        { id: secondLcId, name: "billing", member_count: 1 },
      ],
      statusByLc: {
        [LC_ID]: () => slowStatus,
        [secondLcId]: () =>
          jsonResponse({
            ...knowledgeBaseStatus(),
            initialization_status: "not_initialized",
          }),
      },
      // 两个 LC 的 operation 用不同 step 标识，便于断言「当前 LC 的数据没被换掉」。
      activeByLc: {
        [LC_ID]: () => jsonResponse(knowledgeBaseOperation({ steps: [aStep] })),
        [secondLcId]: () =>
          jsonResponse(knowledgeBaseOperation({ steps: [bStep] })),
      },
    });
    const user = userEvent.setup();

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    // 切到第二个 LC：它的取数先回来（status 未完成 ⇒ 更新按钮保持禁用）。
    await user.click(await screen.findByTestId("lc-selector-billing"));
    const card = await screen.findByTestId("knowledge-base-card");
    const updateButton = within(card).getByRole("button", { name: "知识库更新" });
    await waitFor(() => expect(updateButton).toBeDisabled());

    // 旧 LC 的慢响应此刻才到：它带着「已初始化完成」，若被收下就会解禁更新按钮，
    // 把旧 LC 的状态挂到当前选中的新 LC 上。
    await act(async () => {
      releaseSlowStatus();
      await new Promise((resolve) => {
        setTimeout(resolve, 0);
      });
    });

    expect(updateButton).toBeDisabled();
    // 当前 LC（B）的 operation 没被旧 LC 的数据换掉。
    expect(within(card).getByText("b_step")).toBeInTheDocument();
    expect(within(card).queryByText("a_step")).toBeNull();
  });

  it("status 取数 500 → 错误横幅可见、页面不崩", async () => {
    installKnowledgeBaseFetch({
      statusByLc: {
        [LC_ID]: () =>
          Promise.resolve(
            new Response(
              JSON.stringify({ code: "internal_error", message: "知识库状态读取炸了" }),
              { status: 500 },
            ),
          ),
      },
    });
    const user = userEvent.setup();

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));

    // 失败可见（不被吞掉）：工作台级错误横幅。
    expect(await screen.findByText(/读取知识库状态失败/)).toBeInTheDocument();
    // 且不崩：卡片仍在渲染。
    expect(await screen.findByTestId("knowledge-base-card")).toBeInTheDocument();
  });

  it("启动接口 4xx → 预检失败进卡片、错误横幅可见、页面不崩", async () => {
    installKnowledgeBaseFetch({
      startResponse: () =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              code: "knowledge_base_inputs_incomplete",
              message: "输入模板不完整",
              details: { missing: ["api-scope.md"] },
            }),
            { status: 400 },
          ),
        ),
    });
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "知识库初始化" }));

    // 预检失败落到卡片（缺失文件清单 + 可操作按钮），不吞掉。
    const failure = await screen.findByTestId("knowledge-base-preflight-failure");
    expect(failure).toHaveTextContent("输入模板不完整");
    expect(
      within(failure).getByTestId("knowledge-base-preflight-missing"),
    ).toHaveTextContent("api-scope.md");
    expect(
      within(failure).getByRole("button", { name: "创建输入模板" }),
    ).toBeInTheDocument();
    // 工作台级横幅同时可见（失败不被吞掉）。
    expect(await screen.findByText(/知识库初始化失败/)).toBeInTheDocument();
  });

  it("启动接口返回异常载荷（200 + 空体）→ 不崩、卡片不挂脏 operation", async () => {
    installKnowledgeBaseFetch({
      // 没有 active operation：卡片初始无 operation 徽标。
      activeByLc: {
        [LC_ID]: () =>
          Promise.resolve(
            new Response(
              JSON.stringify({
                code: "knowledge_base_operation_not_found",
                message: "没有进行中的知识库操作",
              }),
              { status: 404 },
            ),
          ),
      },
      // 200 + 空对象：形状守卫必须把它按「无数据」处理，而不是塞进卡片。
      startResponse: () => jsonResponse({}),
    });
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);

    renderWorkbench();

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "知识库初始化" }));

    // 不崩：卡片仍在；脏载荷没有变成 operation 徽标。
    const card = await screen.findByTestId("knowledge-base-card");
    expect(
      within(card).queryByTestId("knowledge-base-operation-status"),
    ).toBeNull();
  });

  it("operation 运行中每 2s 轮询；到终态停止拉取", async () => {
    // shouldAdvanceTime：真实时间自动推进 fake clock，waitFor 的重试间隔可走；
    // 轮询节奏用 advanceTimersByTime 快进驱动，不必真等 2s。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // 受控响应序列：前两次 active 返回 running，之后返回 completed ⇒
    // 终态上屏后轮询必须停止（不得再发起 active 请求）。
    let activeCallCount = 0;
    installKnowledgeBaseFetch({
      activeByLc: {
        [LC_ID]: () => {
          activeCallCount += 1;
          const status = activeCallCount <= 2 ? "running" : "completed";
          return Promise.resolve(
            jsonResponse(knowledgeBaseOperation({ status })),
          );
        },
      },
    });
    renderWorkbench();

    fireEvent.click(await screen.findByTestId("lc-summary-toggle"));
    const card = await screen.findByTestId("knowledge-base-card");
    await waitFor(() =>
      expect(
        within(card).getByTestId("knowledge-base-operation-status"),
      ).toHaveAttribute("data-status", "running"),
    );
    expect(activeCallCount).toBeGreaterThanOrEqual(1);

    // running ⇒ 每 2s 会再次拉取（卡片不得冻结在启动时的 running 快照）。
    const runningBaseline = activeCallCount;
    await act(async () => {
      vi.advanceTimersByTime(2_000);
    });
    await waitFor(() =>
      expect(activeCallCount).toBeGreaterThan(runningBaseline),
    );

    // 拉到 completed ⇒ 终态必须上屏。
    await act(async () => {
      vi.advanceTimersByTime(2_000);
    });
    await waitFor(() =>
      expect(
        within(card).getByTestId("knowledge-base-operation-status"),
      ).toHaveAttribute("data-status", "completed"),
    );

    // 终态 ⇒ 停止拉取：再快进 6s（三个轮询间隔）不得再发起 active 请求。
    const completedBaseline = activeCallCount;
    await act(async () => {
      vi.advanceTimersByTime(6_000);
    });
    expect(activeCallCount).toBe(completedBaseline);
  });

  it("轮询失败静默重试不刷错误横幅；主动动作失败仍报错", async () => {
    // 终审复审 Minor M4：轮询里的取数失败不得每 2s 重打工作台级错误横幅
    // （与被镜像的聚合卡口径一致，静默重试）；横幅只留给用户主动触发的
    // 动作失败——用取消接口 500 证明。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let statusCallCount = 0;
    installKnowledgeBaseFetch({
      statusByLc: {
        [LC_ID]: () => {
          statusCallCount += 1;
          // 首次（抽屉展开的单发取数）成功，卡片进入 running；之后一律
          // 500 ⇒ 轮询持续失败。
          if (statusCallCount === 1) {
            return jsonResponse(knowledgeBaseStatus());
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                code: "internal_error",
                message: "知识库状态读取炸了",
              }),
              { status: 500 },
            ),
          );
        },
      },
      // active 永远 running：轮询不停，失败由 status 侧制造。
      activeByLc: {
        [LC_ID]: () =>
          jsonResponse(knowledgeBaseOperation({ status: "running" })),
      },
      // 取消接口 500：用户主动动作的失败必须如实上报。
      cancelResponse: () =>
        Promise.resolve(
          new Response(
            JSON.stringify({ code: "internal_error", message: "取消失败" }),
            { status: 500 },
          ),
        ),
    });
    renderWorkbench();

    fireEvent.click(await screen.findByTestId("lc-summary-toggle"));
    const card = await screen.findByTestId("knowledge-base-card");
    await waitFor(() =>
      expect(
        within(card).getByTestId("knowledge-base-operation-status"),
      ).toHaveAttribute("data-status", "running"),
    );

    // 快进 3 个轮询间隔：status 侧持续 500（轮询失败发生），但错误横幅
    // 不得被反复刷出。
    await act(async () => {
      vi.advanceTimersByTime(6_000);
    });
    await waitFor(() => expect(statusCallCount).toBeGreaterThan(1));
    expect(screen.queryByText(/读取知识库状态失败/)).toBeNull();

    // 用户主动触发的动作（取消）失败 ⇒ 横幅必须如实上报。
    fireEvent.click(
      within(card).getByRole("button", { name: "取消知识库操作" }),
    );
    expect(await screen.findByText(/取消知识库操作失败/)).toBeInTheDocument();
  });
});
