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
import { useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CockpitShell } from "../components/cockpit/CockpitShell";
import { KnowledgeBasePage } from "./KnowledgeBasePage";
import type { KnowledgeBaseUserInputSummaryDto } from "../api/types/knowledge-base";

// Monaco 编辑器挂载计数：用于验证「折叠文件树不会重挂载编辑器」
// （key 重挂载会清空撤销栈、重置光标与选区、打断输入法）。
const editorMounts = vi.hoisted(() => ({ count: 0 }));

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
  }) => {
    useEffect(() => {
      editorMounts.count += 1;
    }, []);
    return (
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
    );
  },
}));

// CockpitShell（沉浸模式用例）依赖会话观察者投影：单测里给空投影，
// 避免真实轮询发请求。
vi.mock("../hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: () => ({
    records: [],
    inbox: [],
    countedInbox: [],
    watchedSessionIds: [],
    watchSession: () => undefined,
    codingAttemptForSession: () => null,
    displayItems: [],
    actionableCount: 0,
    notificationCandidates: [],
  }),
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

const CONTENTS: Record<string, { content: string; digest: string }> = {
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

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function installFetch(
  overrides: {
    write?: () => Response;
    lint?: () => Response;
    /** 覆盖单文件读取（用于构造乱序/延迟响应）。 */
    read?: (path: string) => Promise<Response>;
    /** 覆盖知识库状态（用于构造前置门拒绝；按 URL 区分 LC 以覆盖换参重跑）。 */
    status?: (url: string) => Response;
    /** 覆盖项目清单（顶栏项目名来源）。 */
    projects?: () => Response;
    /** 覆盖逻辑代码库清单（顶栏 LC 名来源）。 */
    logicalCodebases?: () => Response;
    /** 覆盖输入投影（GET /user-input；默认 404 ⇒ 输入确认区不渲染）。 */
    userInput?: () => Response;
    /** 覆盖输入确认（POST /user-input/confirm；按需翻转夹具状态）。 */
    userInputConfirm?: (body: unknown) => Response;
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
function userInputSummary(
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

/**
 * 渲染 CockpitShell（沉浸模式用例）：initialEntries 的 path 决定外壳是否
 * 命中知识库详情页路由（/workbench/projects/:projectId/logical-codebases/
 * :lcId/knowledge-base）。
 */
function renderShellAtPath(path: string) {
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
    editorMounts.count = 0;
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

  it("header_shows_project_name_logical_codebase_name_and_workbench_entry", async () => {
    installFetch();
    renderPage();

    expect(
      await screen.findByRole("heading", { name: "知识库" }),
    ).toBeInTheDocument();
    const context = await screen.findByTestId("kb-context");
    await waitFor(() => {
      expect(context).toHaveTextContent("示例项目");
      expect(context).toHaveTextContent("核心逻辑代码库");
    });
    // 返回工作台入口指向工作台路由。
    expect(screen.getByTestId("kb-back-to-workbench")).toHaveAttribute(
      "href",
      "/workbench",
    );
    // 状态徽标来自既有 status 投影（已初始化）。
    expect(screen.getByTestId("kb-status")).toHaveTextContent("已初始化");
  });

  it("header_falls_back_to_ids_when_names_are_unavailable", async () => {
    installFetch({
      projects: () =>
        jsonResponse({ code: "not_found", message: "项目清单不可用" }, 404),
      logicalCodebases: () =>
        jsonResponse({ code: "not_found", message: "LC 清单不可用" }, 404),
    });
    renderPage();

    const context = await screen.findByTestId("kb-context");
    await waitFor(() => {
      expect(context).toHaveTextContent("p1");
      expect(context).toHaveTextContent("lc1");
    });
    // 取名失败不得让副标题留空。
    expect(context.textContent?.trim()).not.toBe("");
  });

  it("status_badge_reports_preflight_gate_rejection", async () => {
    installFetch({
      status: () =>
        jsonResponse(
          {
            code: "knowledge_base_requires_aggregate_initialization",
            message: "请先完成聚合初始化",
          },
          403,
        ),
    });
    renderPage();

    // 前置门未满足：显示「前置未满足」徽标（不臆造 status 字段）。
    expect(await screen.findByTestId("kb-status")).toHaveTextContent(
      "前置未满足",
    );
  });

  it("layout_fills_available_height_with_two_columns", async () => {
    installFetch();
    renderPage();

    const workspace = await screen.findByTestId("kb-workspace");
    expect(workspace.className).toContain("h-full");
    expect(workspace.className).toContain("min-h-0");
    // 左：文件树固定宽（不随右区伸展）。
    const treePanel = screen.getByTestId("kb-file-tree-panel");
    expect(treePanel.className).toContain("shrink-0");
    // 右：编辑/预览自适应并吃满可用高度。
    const editorPanel = screen.getByTestId("kb-editor-panel");
    expect(editorPanel.className).toContain("flex-1");
    expect(editorPanel.className).toContain("min-h-0");
    const tabpanel = screen.getByTestId("kb-tabpanel");
    expect(tabpanel.className).toContain("flex-1");
    expect(tabpanel.className).toContain("min-h-0");
    // 编辑器宿主拉满（高度不由固定像素决定 ⇒ 下半屏不留空白）。
    const editorHost = screen.getByTestId("kb-editor-host");
    expect(editorHost.className).toContain("h-full");
  });

  it("file_tree_can_be_collapsed", async () => {
    installFetch();
    renderPage();

    await screen.findByTestId("kb-file-tree");
    fireEvent.click(screen.getByTestId("kb-tree-toggle"));
    expect(screen.queryByTestId("kb-file-tree")).toBeNull();
    fireEvent.click(screen.getByTestId("kb-tree-toggle"));
    expect(await screen.findByTestId("kb-file-tree")).toBeInTheDocument();
  });

  it("collapsing_file_tree_does_not_remount_editor", async () => {
    installFetch();
    renderPage();

    // Monaco 默认开启 automaticLayout ⇒ 折叠文件树无需重挂载编辑器
    // （重挂载会清空撤销栈、重置光标与选区、打断输入法）。
    await screen.findByTestId("monaco-editor");
    await waitFor(() => {
      expect(editorMounts.count).toBe(1);
    });

    fireEvent.click(screen.getByTestId("kb-tree-toggle"));
    await waitFor(() => {
      expect(screen.queryByTestId("kb-file-tree")).toBeNull();
    });
    // 折叠后编辑器不得被卸载重挂（挂载次数保持 1）。
    expect(editorMounts.count).toBe(1);

    fireEvent.click(screen.getByTestId("kb-tree-toggle"));
    await waitFor(() => {
      expect(screen.getByTestId("kb-file-tree")).toBeInTheDocument();
    });
    expect(editorMounts.count).toBe(1);
  });

  it("preflight_badge_clears_when_new_logical_codebase_status_succeeds", async () => {
    installFetch({
      status: (url) =>
        url.includes("/lc1/")
          ? jsonResponse(
              {
                code: "knowledge_base_requires_aggregate_initialization",
                message: "请先完成聚合初始化",
              },
              403,
            )
          : jsonResponse({
              exists: true,
              schema_version: "4.0",
              initialization_status: "complete",
              global_validation: "passed",
              completed_stages: ["base-info"],
              skipped_stages: [],
              skills_available: true,
            }),
    });
    const router = renderPage();

    expect(await screen.findByTestId("kb-status")).toHaveTextContent(
      "前置未满足",
    );

    // 同路由仅换 LC 参数：组件不卸载，副作用重跑；成功路径须清零徽标。
    await act(async () => {
      await router.navigate({ to: "/knowledge-base/p1/lc2" });
    });

    await waitFor(() => {
      expect(screen.getByTestId("kb-status")).toHaveTextContent("已初始化");
    });
    expect(screen.getByTestId("kb-status")).not.toHaveTextContent("前置未满足");
  });

  it("cockpit_shell_hides_global_widgets_on_knowledge_base_route", async () => {
    installFetch();
    renderShellAtPath(
      "/workbench/projects/p1/logical-codebases/lc1/knowledge-base",
    );

    await screen.findByTestId("shell-child");
    // 沉浸模式：全局设置入口（及其兜底浮层）不渲染。
    expect(screen.queryByTestId("cockpit-settings-trigger")).toBeNull();
    expect(screen.queryByTestId("cockpit-settings-fallback")).toBeNull();
  });

  it("cockpit_shell_keeps_global_widgets_on_other_routes", async () => {
    installFetch();
    renderShellAtPath("/workbench");

    await screen.findByTestId("shell-child");
    // 非知识库路由：既有行为零变化（全局设置入口仍在）。
    expect(screen.getByTestId("cockpit-settings-trigger")).toBeInTheDocument();
  });

  it("user_input_panel_lists_files_with_badges_and_pending_hint", async () => {
    installFetch({
      userInput: () =>
        jsonResponse(
          userInputSummary([
            { path: "user-input/goal.md", confirmed: true },
            { path: "user-input/scope.md" },
          ]),
        ),
    });
    renderPage();

    const panel = await screen.findByTestId("kb-user-input-panel");
    expect(panel).toBeInTheDocument();
    const badgeOf = (path: string) =>
      within(panel)
        .getAllByTestId("kb-user-input-badge")
        .find((node) => node.getAttribute("data-path") === path);
    await waitFor(() => {
      expect(badgeOf("user-input/goal.md")).toHaveTextContent("已确认");
      expect(badgeOf("user-input/scope.md")).toHaveTextContent("待确认");
    });
    // 未确认时提示体现「待确认 N 份」（N = 总数 - 已确认数，含未填份额）。
    expect(within(panel).getByTestId("kb-user-input-summary")).toHaveTextContent(
      "待确认 1 份",
    );
  });

  it("saving_confirmed_input_file_invalidates_and_reconfirms", async () => {
    let goalConfirmed = true;
    let goalDigest = "digest-goal";
    const { calls } = installFetch({
      userInput: () =>
        jsonResponse(
          userInputSummary([
            { path: "user-input/goal.md", confirmed: goalConfirmed },
          ]),
        ),
      userInputConfirm: () => {
        goalConfirmed = true;
        return jsonResponse({ confirmed: ["user-input/goal.md"] });
      },
      write: () => {
        goalConfirmed = false;
        goalDigest = "digest-goal-v2";
        return jsonResponse({ path: "user-input/goal.md", digest: goalDigest });
      },
      read: (path) =>
        Promise.resolve(
          jsonResponse({
            path,
            content: CONTENTS[path]?.content ?? "# 内容",
            digest:
              path === "user-input/goal.md"
                ? goalDigest
                : (CONTENTS[path]?.digest ?? "digest-x"),
          }),
        ),
    });
    renderPage();

    // 点输入文件：右侧编辑器打开并可直接编辑。
    fireEvent.click(await screen.findByTestId("kb-user-input-open"));
    await waitForEditorValue("# 目标");
    fireEvent.change(screen.getByTestId("kb-editor"), {
      target: { value: "# 目标（已改）" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    // 保存后该份确认按 digest 自动失效：徽标回到待确认并提示需重新确认。
    expect(await screen.findByTestId("kb-user-input-invalidated")).toHaveTextContent(
      "已修改，需重新确认",
    );
    await waitFor(() => {
      const badge = screen
        .getAllByTestId("kb-user-input-badge")
        .find((node) => node.getAttribute("data-path") === "user-input/goal.md");
      expect(badge).toHaveTextContent("待确认");
    });

    // 重新确认携带当前读到的 digest；成功后提示消失、徽标回到已确认。
    fireEvent.click(screen.getByTestId("kb-user-input-confirm"));
    const confirmPost = await waitFor(() => {
      const post = calls.find(
        (call) => call.method === "POST" && call.url.includes("/user-input/confirm"),
      );
      expect(post).toBeDefined();
      return post;
    });
    expect(confirmPost?.body).toEqual({
      path: "user-input/goal.md",
      digest: "digest-goal-v2",
    });
    await waitFor(() => {
      expect(screen.queryByTestId("kb-user-input-invalidated")).toBeNull();
    });
    await waitFor(() => {
      const badge = screen
        .getAllByTestId("kb-user-input-badge")
        .find((node) => node.getAttribute("data-path") === "user-input/goal.md");
      expect(badge).toHaveTextContent("已确认");
    });
  });
});
