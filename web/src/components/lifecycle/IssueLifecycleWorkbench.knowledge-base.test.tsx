// Task 9.5：知识库卡片的宿主接线测试。
// 覆盖四件事：
//   1. 抽屉展开后卡片拿到 status（GET .../knowledge-base/status）与 active
//      operation（GET .../knowledge-base/initializations/active）——不再空态；
//   2. 点「知识库初始化」发起 POST .../knowledge-base/initializations，
//      body 带 kind=bootstrap 与每次唯一的幂等键；
//   3. 点「知识库更新」把选中的变更包路径透传进 POST（package_path 不能丢——
//      丢了后端拿不到变更包，门禁形同虚设）；
//   4. 点「查看知识库详细内容」跳 Task 10 的详情页路由。
import { render, screen, waitFor, within } from "@testing-library/react";
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
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

const PROJECT_ID = "project_0001";
const LC_ID = "lc_0001";
const KB_API = `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/knowledge-base`;
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
 */
function installKnowledgeBaseFetch() {
  const base = lifecycleFetch({
    projects: [projectRecord(PROJECT_ID, "Aria")],
    logicalCodebases: [{ id: LC_ID, name: "platform", member_count: 1 }],
  });
  const startBodies: Array<Record<string, unknown>> = [];
  const fetchMock = vi.fn(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      if (url === `${KB_API}/status`) {
        return jsonResponse(knowledgeBaseStatus());
      }
      if (url === `${KB_API}/initializations/active`) {
        return jsonResponse(knowledgeBaseOperation());
      }
      if (url === `${KB_API}/change-packages`) {
        return jsonResponse({ packages: [knowledgeBasePackage()] });
      }
      if (url === `${KB_API}/initializations` && method === "POST") {
        startBodies.push(
          JSON.parse(String(init?.body)) as Record<string, unknown>,
        );
        return jsonResponse(knowledgeBaseOperation({ status: "running" }));
      }
      if (url === `${KB_API}/bootstrap-inputs` && method === "POST") {
        return jsonResponse({ created: ["api-scope.md"] });
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
});
