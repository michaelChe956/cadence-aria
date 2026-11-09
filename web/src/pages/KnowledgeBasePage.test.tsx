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
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
    options,
  }: {
    value?: string;
    onChange?: (value: string | undefined) => void;
    options?: { readOnly?: boolean };
  }) => (
    <div
      data-testid="monaco-editor"
      data-readonly={options?.readOnly ? "true" : "false"}
    >
      <textarea
        data-testid="kb-editor"
        value={value ?? ""}
        onChange={(event) => onChange?.(event.target.value)}
      />
    </div>
  ),
}));

// 文件树夹具：path 一律是**知识库根相对**口径（与后端 files.rs / HTTP 层一致，
// 如 base-info.md、user-input/...、manifest.yaml——不是聚合根相对）。
const FILES = [
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
];

const CONTENTS: Record<string, { content: string; digest: string }> = {
  "base-info.md": { content: "# 基础信息", digest: "digest-base" },
  "product.md": { content: "# 产品信息", digest: "digest-product" },
  "manifest.yaml": { content: "schema_version: 4.0", digest: "digest-manifest" },
  "evidence/relation-graph.yaml": { content: "nodes: []", digest: "digest-graph" },
  "user-input/updates/CHANGE-1/code-change.md": {
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
    /** 覆盖单文件读取（用于构造乱序/延迟响应）。 */
    read?: (path: string) => Promise<Response>;
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
      if (overrides.read) {
        return overrides.read(path);
      }
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

/** 取文件节点所属的目录分组路径（文件项所在 group 的父 li 上的目录标题）。 */
function dirPathOf(fileNode: HTMLElement): string | null {
  const item = fileNode.closest("li");
  const group = item?.parentElement;
  const dirItem = group?.parentElement;
  return (
    dirItem?.querySelector('[data-testid="kb-dir-node"]')?.getAttribute("data-path") ??
    null
  );
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
    expect(paths).toContain("base-info.md");
    expect(paths).toContain("user-input/updates/CHANGE-1/code-change.md");
    expect(await screen.findByTestId("kb-status")).toHaveTextContent("已初始化");
  });

  it("renders_directory_groups_in_file_tree", async () => {
    installFetch();
    renderPage();

    await screen.findByTestId("kb-file-tree");

    // 顶层目录必须是树根的直接子节点（此前目录只进 Map、未挂根容器 ⇒ 目录分组整体不可达）。
    const treeRoot = screen.getByRole("tree");
    const userInputDir = screen
      .getAllByTestId("kb-dir-node")
      .find((node) => node.getAttribute("data-path") === "user-input");
    expect(userInputDir).toBeDefined();
    const userInputItem = userInputDir?.closest("li");
    expect(userInputItem?.parentElement).toBe(treeRoot);

    // 目录分组下挂有文件节点（嵌套层级：user-input → updates → CHANGE-* → 文件）。
    const updatesDir = screen
      .getAllByTestId("kb-dir-node")
      .find((node) => node.getAttribute("data-path") === "user-input/updates");
    expect(updatesDir).toBeDefined();
    expect(
      within(userInputItem as HTMLElement)
        .getAllByTestId("kb-file-node")
        .map((node) => node.getAttribute("data-path")),
    ).toEqual(
      expect.arrayContaining([
        "user-input/updates/CHANGE-1/code-change.md",
        "user-input/updates/CHANGE-2/code-change.md",
      ]),
    );

    // 同名文件（code-change.md）必须按所在目录区分。
    const codeChangeButtons = screen.getAllByRole("button", { name: "code-change.md" });
    expect(codeChangeButtons).toHaveLength(2);
    expect(codeChangeButtons.map((button) => dirPathOf(button))).toEqual(
      expect.arrayContaining([
        "user-input/updates/CHANGE-1",
        "user-input/updates/CHANGE-2",
      ]),
    );

    // 根层文件不隶属于任何目录分组。
    const overview = screen.getByRole("button", { name: "overview.md" });
    expect(overview.closest('[data-testid="kb-dir-node"]')).toBeNull();
  });

  it("read_failure_clears_content_and_blocks_save", async () => {
    const { calls } = installFetch();
    renderPage();

    await waitForEditorValue("# 基础信息");

    fireEvent.click(screen.getByRole("button", { name: "missing.md" }));

    const notice = await screen.findByTestId("kb-notice");
    await waitFor(() => {
      expect(notice).toHaveTextContent("读取文件失败");
    });
    // 读取失败必须清空内容状态（不得残留上一文件的内容）。
    await waitFor(() => {
      expect((screen.getByTestId("kb-editor") as HTMLTextAreaElement).value).toBe("");
    });
    // 无读时摘要 ⇒ 保存禁用（不得无 baseDigest 盲覆盖写）。
    const saveButton = screen.getByRole("button", { name: "保存" });
    await waitFor(() => {
      expect(saveButton).toBeDisabled();
    });
    fireEvent.click(saveButton);
    expect(calls.filter((call) => call.method === "PUT")).toHaveLength(0);
  });

  it("stale_read_response_does_not_overwrite_current_file", async () => {
    let releaseBaseInfo: ((response: Response) => void) | null = null;
    const baseInfoPending = new Promise<Response>((resolve) => {
      releaseBaseInfo = resolve;
    });
    installFetch({
      read: (path) =>
        path.endsWith("base-info.md")
          ? baseInfoPending
          : Promise.resolve(
              jsonResponse({
                path,
                content: "# 产品信息",
                digest: "digest-product",
              }),
            ),
    });
    renderPage();

    // 首屏自动打开 base-info.md（读取挂起），随后切换到 product.md。
    await screen.findByTestId("kb-file-tree");
    fireEvent.click(screen.getByRole("button", { name: "product.md" }));
    await waitForEditorValue("# 产品信息");

    // 先到的 product.md 响应已落地；此时才到达的 base-info.md 响应属于过期响应。
    await act(async () => {
      releaseBaseInfo?.(
        jsonResponse({
          path: "base-info.md",
          content: "# 基础信息",
          digest: "digest-base",
        }),
      );
      await baseInfoPending;
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect((screen.getByTestId("kb-editor") as HTMLTextAreaElement).value).toBe(
      "# 产品信息",
    );
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
      path: "base-info.md",
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

    // 两个变更包下存在同名文件 ⇒ 按完整路径定位目标按钮。
    const codeChange = (
      await screen.findAllByTestId("kb-file-node")
    ).find(
      (node) =>
        node.getAttribute("data-path") ===
        "user-input/updates/CHANGE-1/code-change.md",
    );
    expect(codeChange).toBeDefined();
    fireEvent.click(codeChange as HTMLElement);

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

  it("machine_managed_file_rendered_read_only", async () => {
    const { calls } = installFetch();
    renderPage();

    await waitForEditorValue("# 基础信息");

    // 打开机器管理文件 manifest.yaml：内容仍可查看，但编辑器只读
    // （不向 MonacoEditor 传 onChange ⇒ 组件内建 readOnly 生效）。
    fireEvent.click(screen.getByRole("button", { name: "manifest.yaml" }));
    await waitForEditorValue("schema_version: 4.0");
    expect(screen.getByTestId("monaco-editor")).toHaveAttribute(
      "data-readonly",
      "true",
    );
    // 保存入口不可用，且点击不得发出写请求。
    const saveButton = screen.getByRole("button", { name: "保存" });
    expect(saveButton).toBeDisabled();
    fireEvent.click(saveButton);
    expect(calls.filter((call) => call.method === "PUT")).toHaveLength(0);
    // 只读说明可见。
    expect(screen.getByTestId("kb-readonly-hint")).toHaveTextContent(
      "机器管理文件，只读",
    );

    // 切回可编辑文件：恢复可写 + 可保存，说明消失。
    fireEvent.click(screen.getByRole("button", { name: "product.md" }));
    await waitForEditorValue("# 产品信息");
    expect(screen.getByTestId("monaco-editor")).toHaveAttribute(
      "data-readonly",
      "false",
    );
    expect(screen.queryByTestId("kb-readonly-hint")).toBeNull();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });
});
