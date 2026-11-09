// Task 10：LC 知识库详情页（文件树 + 可写编辑器 + 预览 + 保存）组件测试。
// 路由正确性由 router.tsx 的路由注册与 `pnpm -C web build` 的 tsc 检查共同保证。
import {
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useParams,
} from "@tanstack/react-router";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { KnowledgeBasePage } from "./KnowledgeBasePage";

vi.mock("@monaco-editor/react", () => ({
  // ★`monaco-local-setup.ts` 在模块加载期调用 `loader.config` ⇒ mock 必须带 loader。
  loader: {
    config: vi.fn(),
  },
  Editor: ({
    value,
    onChange,
  }: {
    value?: string;
    onChange?: (value: string | undefined) => void;
  }) => (
    <div data-testid="monaco-editor">
      <textarea
        data-testid="kb-editor"
        value={value ?? ""}
        onChange={(event) => onChange?.(event.target.value)}
      />
    </div>
  ),
}));

const FILES = [
  { path: "cadence/knowledge-base", is_dir: true, size: 0, editable: false },
  { path: "cadence/knowledge-base/base-info.md", is_dir: false, size: 12, editable: true },
  { path: "cadence/knowledge-base/product.md", is_dir: false, size: 12, editable: true },
  { path: "cadence/knowledge-base/user-input", is_dir: true, size: 0, editable: false },
  {
    path: "cadence/knowledge-base/user-input/updates",
    is_dir: true,
    size: 0,
    editable: false,
  },
  {
    path: "cadence/knowledge-base/user-input/updates/CHANGE-1",
    is_dir: true,
    size: 0,
    editable: false,
  },
  {
    path: "cadence/knowledge-base/user-input/updates/CHANGE-1/code-change.md",
    is_dir: false,
    size: 12,
    editable: true,
  },
];

const CONTENTS: Record<string, { content: string; digest: string }> = {
  "cadence/knowledge-base/base-info.md": {
    content: "# 基础信息",
    digest: "digest-base",
  },
  "cadence/knowledge-base/product.md": {
    content: "# 产品信息",
    digest: "digest-product",
  },
  "cadence/knowledge-base/user-input/updates/CHANGE-1/code-change.md": {
    content: "# 代码变更",
    digest: "digest-change",
  },
};

type FetchCall = { url: string; method: string; body?: unknown };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function installFetch(
  overrides: {
    write?: () => Response;
    lint?: () => Response;
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
        : jsonResponse({ path: "cadence/knowledge-base/base-info.md", digest: "digest-new" });
    }
    if (url.includes("/knowledge-base/status")) {
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
      return jsonResponse({ nodes: FILES });
    }
    if (url.includes("/change-packages/lint")) {
      return overrides.lint
        ? overrides.lint()
        : jsonResponse({ violations: [], fields: [] });
    }
    if (url.includes("/knowledge-base/file")) {
      const path = decodeURIComponent(
        new URL(url, "http://localhost").searchParams.get("path") ?? "",
      );
      const entry = CONTENTS[path];
      if (!entry) {
        return jsonResponse({ code: "knowledge_base_file_not_found", message: "文件不存在" }, 404);
      }
      return jsonResponse({ path, content: entry.content, digest: entry.digest });
    }
    return jsonResponse({ code: "not_found", message: `未知请求 ${url}` }, 404);
  });
  vi.stubGlobal("fetch", fetchMock);
  return { calls };
}

function createTestRouter() {
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

function renderPage() {
  const router = createTestRouter();
  render(<RouterProvider router={router} />);
  return router;
}

async function waitForEditorValue(expected: string) {
  const editor = await screen.findByTestId("kb-editor");
  await waitFor(() => {
    expect((editor as HTMLTextAreaElement).value).toBe(expected);
  });
}

describe("KnowledgeBasePage", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders_file_tree_from_status", async () => {
    installFetch();
    renderPage();

    const tree = await screen.findByTestId("kb-file-tree");
    expect(tree).toBeInTheDocument();
    const paths = screen
      .getAllByTestId("kb-file-node")
      .map((node) => node.getAttribute("data-path"));
    expect(paths).toContain("cadence/knowledge-base/base-info.md");
    expect(paths).toContain(
      "cadence/knowledge-base/user-input/updates/CHANGE-1/code-change.md",
    );
    expect(await screen.findByTestId("kb-status")).toHaveTextContent("已初始化");
  });

  it("switching_file_loads_content_into_editor", async () => {
    installFetch();
    renderPage();

    await waitForEditorValue("# 基础信息");
    fireEvent.click(screen.getByRole("button", { name: "product.md" }));

    await waitForEditorValue("# 产品信息");
  });

  it("save_writes_file_with_digest", async () => {
    const { calls } = installFetch();
    renderPage();

    await waitForEditorValue("# 基础信息");
    fireEvent.change(screen.getByTestId("kb-editor"), {
      target: { value: "# 基础信息（已改）" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((call) => call.method === "PUT")).toBe(true);
    });
    const put = calls.find((call) => call.method === "PUT");
    expect(put?.body).toEqual({
      path: "cadence/knowledge-base/base-info.md",
      content: "# 基础信息（已改）",
      base_digest: "digest-base",
    });
    expect(await screen.findByTestId("kb-notice")).toHaveTextContent("已保存");
  });

  it("conflict_409_shows_message", async () => {
    const { calls } = installFetch({
      write: () =>
        jsonResponse(
          { code: "knowledge_base_file_conflict", message: "文件内容已被修改" },
          409,
        ),
    });
    renderPage();

    await waitForEditorValue("# 基础信息");
    fireEvent.change(screen.getByTestId("kb-editor"), {
      target: { value: "# 基础信息（冲突）" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    const notice = await screen.findByTestId("kb-notice");
    await waitFor(() => {
      expect(notice).toHaveTextContent("内容已被修改");
    });
    // 冲突不得被静默重试覆盖。
    expect(calls.filter((call) => call.method === "PUT")).toHaveLength(1);
  });

  it("operation_running_409_shows_message", async () => {
    installFetch({
      write: () =>
        jsonResponse(
          { code: "knowledge_base_operation_running", message: "知识库操作进行中" },
          409,
        ),
    });
    renderPage();

    await waitForEditorValue("# 基础信息");
    fireEvent.change(screen.getByTestId("kb-editor"), {
      target: { value: "# 基础信息（操作中）" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    const notice = await screen.findByTestId("kb-notice");
    await waitFor(() => {
      expect(notice).toHaveTextContent("操作进行中");
    });
  });

  it("change_package_fields_show_unfilled_markers", async () => {
    installFetch({
      lint: () =>
        jsonResponse({
          violations: [],
          fields: [
            { document: "code-change.md", field: "Merge Request 地址或编号", filled: false },
            { document: "code-change.md", field: "影响范围说明", filled: true },
          ],
        }),
    });
    renderPage();

    fireEvent.click(
      await screen.findByRole("button", { name: "code-change.md" }),
    );

    expect(await screen.findByText("未填")).toBeInTheDocument();
    expect(screen.getByText("Merge Request 地址或编号")).toBeInTheDocument();
    expect(screen.getByText("已填")).toBeInTheDocument();
  });

  it("unsaved_changes_guarded", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    installFetch();
    const router = renderPage();

    await waitForEditorValue("# 基础信息");
    fireEvent.change(screen.getByTestId("kb-editor"), {
      target: { value: "# 基础信息（未保存）" },
    });

    await router.navigate({ to: "/other" });

    await waitFor(() => {
      expect(confirmSpy).toHaveBeenCalled();
    });
    expect(String(confirmSpy.mock.calls[0]?.[0] ?? "")).toContain("未保存");
  });
});
