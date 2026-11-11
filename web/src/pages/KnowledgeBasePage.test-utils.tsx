// LC 知识库详情页组件测试的**共享夹具**（文件树 / 内容 / fetch 打桩 / 路由）。
//
// 详情页的用例分成两支（`KnowledgeBasePage.test.tsx` 主体用例与
// `KnowledgeBasePage.inputFlow.test.tsx` 输入流用例），共享同一套夹具——
// 复制两份会让「夹具漂移」变成隐形陷阱（一边改了另一边没改）。
//
// `vi.mock`（monaco / observers）留在各测试文件：mock 注册是**按文件**的，
// 放到本模块里对引用它的测试文件不生效。
import {
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useParams,
} from "@tanstack/react-router";
import { render, screen, waitFor } from "@testing-library/react";
import { expect, vi } from "vitest";
import { CockpitShell } from "../components/cockpit/CockpitShell";
import { KnowledgeBasePage } from "./KnowledgeBasePage";
import type { KnowledgeBaseUserInputSummaryDto } from "../api/types/knowledge-base";

// CockpitShell（沉浸模式用例）依赖会话观察者投影：单测里给空投影，
// 避免真实轮询发请求。
// 文件树夹具：path 一律是**知识库根相对**口径（与后端 files.rs / HTTP 层一致，
// 如 base-info.md、user-input/...、manifest.yaml——不是聚合根相对）。
export const FILES = [
  { path: "base-info.md", is_dir: false, size: 12, editable: true },
  { path: "product.md", is_dir: false, size: 12, editable: true },
  // 机器管理文件：editable=false ⇒ 只读呈现（编辑器只读 + 保存禁用 + 说明）。
  { path: "manifest.yaml", is_dir: false, size: 48, editable: false },
  { path: "evidence", is_dir: true, size: 0, editable: false },
  {
    path: "evidence/relation-graph.yaml",
    is_dir: false,
    size: 64,
    editable: false,
  },
  { path: "user-input", is_dir: true, size: 0, editable: false },
  { path: "user-input/updates", is_dir: true, size: 0, editable: false },
  {
    path: "user-input/updates/CHANGE-1",
    is_dir: true,
    size: 0,
    editable: false,
  },
  {
    path: "user-input/updates/CHANGE-1/code-change.md",
    is_dir: false,
    size: 12,
    editable: true,
  },
  {
    // 第二个变更包：与 CHANGE-1 同名文件，用于验证「同名文件按目录区分」。
    path: "user-input/updates/CHANGE-2",
    is_dir: true,
    size: 0,
    editable: false,
  },
  {
    path: "user-input/updates/CHANGE-2/code-change.md",
    is_dir: false,
    size: 12,
    editable: true,
  },
  {
    // 根层文件（父路径为 null）：必须直接挂在树根，不得被折叠进任何目录分组。
    path: "overview.md",
    is_dir: false,
    size: 8,
    editable: true,
  },
  {
    // 读取必然失败（不在 CONTENTS 中 ⇒ 404）：用于验证读取失败后的状态清理。
    path: "missing.md",
    is_dir: false,
    size: 0,
    editable: true,
  },
  {
    // 输入确认区用例：9 份输入之一（可编辑 ⇒ 面板点击后在右侧编辑器打开）。
    path: "user-input/goal.md",
    is_dir: false,
    size: 10,
    editable: true,
  },
  {
    path: "user-input/scope.md",
    is_dir: false,
    size: 10,
    editable: true,
  },
];

export const CONTENTS: Record<string, { content: string; digest: string }> = {
  "base-info.md": { content: "# 基础信息", digest: "digest-base" },
  "product.md": { content: "# 产品信息", digest: "digest-product" },
  "manifest.yaml": { content: "schema_version: 4.0", digest: "digest-manifest" },
  "evidence/relation-graph.yaml": { content: "nodes: []", digest: "digest-graph" },
  "user-input/updates/CHANGE-1/code-change.md": {
    content: "# 代码变更",
    digest: "digest-change",
  },
  "user-input/goal.md": { content: "# 目标", digest: "digest-goal" },
  "user-input/scope.md": { content: "# 范围", digest: "digest-scope" },
};

type FetchCall = { url: string; method: string; body?: unknown };

export function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

export function installFetch(
  overrides: {
    write?: () => Response;
    lint?: () => Response;
    /** 覆盖单文件读取（用于构造乱序/延迟响应）。 */
    read?: (path: string) => Promise<Response>;
    /** 覆盖知识库状态（用于构造前置门拒绝；按 URL 区分 LC 以覆盖换参重跑）。 */
    status?: (url: string) => Response;
    /** 覆盖知识库文件树（用于构造前置门拒绝下的只读浏览面）。 */
    files?: () => Response;
    /** 覆盖项目清单（顶栏项目名来源）。 */
    projects?: () => Response;
    /** 覆盖逻辑代码库清单（顶栏 LC 名来源）。 */
    logicalCodebases?: () => Response;
    /** 覆盖输入投影（GET /user-input；默认 404 ⇒ 输入确认区不渲染）。 */
    userInput?: () => Response;
    /** 覆盖输入确认（POST /user-input/confirm；按需翻转夹具状态）。 */
    userInputConfirm?: (body: unknown) => Response;
    /** 覆盖建库启动（POST /knowledge-base/initializations）。 */
    initializations?: (body: unknown) => Response;
    /** 覆盖当前 operation 投影（GET /knowledge-base/initializations/active）。 */
    activeOperation?: () => Response;
  } = {},
): { calls: FetchCall[] } {
  const calls: FetchCall[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    calls.push({
      url,
      method,
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    });
    if (method === "PUT") {
      return overrides.write
        ? overrides.write()
        : jsonResponse({ path: "base-info.md", digest: "digest-new" });
    }
    if (url.includes("/knowledge-base/status")) {
      if (overrides.status) {
        return overrides.status(url);
      }
      return jsonResponse({
        exists: true,
        schema_version: "4.0",
        initialization_status: "complete",
        global_validation: "passed",
        completed_stages: ["base-info"],
        skipped_stages: [],
        skills_available: true,
      });
    }
    if (url.includes("/knowledge-base/files")) {
      if (overrides.files) {
        return overrides.files();
      }
      return jsonResponse({ nodes: FILES });
    }
    if (url.includes("/change-packages/lint")) {
      return overrides.lint
        ? overrides.lint()
        : jsonResponse({ violations: [], fields: [] });
    }
    if (url.includes("/knowledge-base/user-input/confirm")) {
      return overrides.userInputConfirm
        ? overrides.userInputConfirm(calls.at(-1)?.body)
        : jsonResponse({ confirmed: [] });
    }
    if (url.includes("/knowledge-base/initializations/active")) {
      return overrides.activeOperation
        ? overrides.activeOperation()
        : jsonResponse(
            { code: "knowledge_base_operation_not_found", message: "无 operation" },
            404,
          );
    }
    if (url.includes("/knowledge-base/initializations")) {
      return overrides.initializations
        ? overrides.initializations(calls.at(-1)?.body)
        : jsonResponse({
            operation_id: "kb_op_0002",
            project_id: "p1",
            logical_codebase_id: "lc1",
            kind: "bootstrap",
            status: "running",
            steps: [],
            error: null,
            created_at: "2026-01-01T00:00:00Z",
            started_at: null,
            completed_at: null,
          });
    }
    if (url.includes("/knowledge-base/user-input")) {
      if (overrides.userInput) {
        return overrides.userInput();
      }
      // 默认不提供输入投影（404）：既有用例不渲染输入确认区，断言面稳定。
      return jsonResponse({ code: "not_found", message: "输入投影不可用" }, 404);
    }
    if (url.includes("/knowledge-base/file")) {
      const path = decodeURIComponent(
        new URL(url, "http://localhost").searchParams.get("path") ?? "",
      );
      if (overrides.read) {
        return overrides.read(path);
      }
      const entry = CONTENTS[path];
      if (!entry) {
        return jsonResponse({ code: "knowledge_base_file_not_found", message: "文件不存在" }, 404);
      }
      return jsonResponse({ path, content: entry.content, digest: entry.digest });
    }
    // 顶栏取名用的既有只读端点：项目清单与逻辑代码库清单。
    if (/^\/api\/projects\/[^/]+\/logical-codebases$/.test(url)) {
      if (overrides.logicalCodebases) {
        return overrides.logicalCodebases();
      }
      return jsonResponse({
        logical_codebases: [{ id: "lc1", name: "核心逻辑代码库", aggregate_root: "repos/core" }],
      });
    }
    if (url === "/api/projects") {
      if (overrides.projects) {
        return overrides.projects();
      }
      return jsonResponse({
        projects: [
          {
            project_id: "p1",
            name: "示例项目",
            description: null,
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
            last_opened_at: null,
          },
        ],
      });
    }
    return jsonResponse({ code: "not_found", message: `未知请求 ${url}` }, 404);
  });
  vi.stubGlobal("fetch", fetchMock);
  return { calls };
}
/** 构造 GET /user-input 汇总投影：默认已填写且未确认（filled ⇒ pending）。 */
export function userInputSummary(
  files: { path: string; filled?: boolean; confirmed?: boolean }[],
): KnowledgeBaseUserInputSummaryDto {
  const rows = files.map((file) => {
    const filled = file.filled ?? true;
    const confirmed = file.confirmed ?? false;
    return { path: file.path, filled, confirmed, pending: filled && !confirmed };
  });
  return {
    files: rows,
    confirmed_count: rows.filter((row) => row.confirmed).length,
    total_count: rows.length,
  };
}

export function createTestRouter() {
  const rootRoute = createRootRoute({ component: () => <Outlet /> });
  const knowledgeBaseRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/knowledge-base/$projectId/$lcId",
    component: () => {
      const { projectId, lcId } = useParams({ strict: false });
      return <KnowledgeBasePage projectId={projectId} lcId={lcId} />;
    },
  });
  const otherRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/other",
    component: () => <div data-testid="other-page">其它页</div>,
  });
  const routeTree = rootRoute.addChildren([knowledgeBaseRoute, otherRoute]);
  return createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: ["/knowledge-base/p1/lc1"] }),
  });
}

export function renderPage() {
  const router = createTestRouter();
  render(<RouterProvider router={router} />);
  return router;
}

/**
 * 渲染 CockpitShell（沉浸模式用例）：initialEntries 的 path 决定外壳是否
 * 命中知识库详情页路由（/workbench/projects/:projectId/logical-codebases/
 * :lcId/knowledge-base）。
 */
export function renderShellAtPath(path: string) {
  const rootRoute = createRootRoute({
    component: () => (
      <CockpitShell>
        <div data-testid="shell-child">子页面</div>
      </CockpitShell>
    ),
  });
  const router = createRouter({
    routeTree: rootRoute,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

export async function waitForEditorValue(expected: string) {
  const editor = await screen.findByTestId("kb-editor");
  await waitFor(() => {
    expect((editor as HTMLTextAreaElement).value).toBe(expected);
  });
}

/** 取文件节点所属的目录分组路径（文件项所在 group 的父 li 上的目录标题）。 */
export function dirPathOf(fileNode: HTMLElement): string | null {
  const item = fileNode.closest("li");
  const group = item?.parentElement;
  const dirItem = group?.parentElement;
  return (
    dirItem?.querySelector('[data-testid="kb-dir-node"]')?.getAttribute("data-path") ??
    null
  );
}
