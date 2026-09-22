import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RealProviderName } from "../../api/types";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { providerHealthSnapshot } from "../../state/provider-availability-test-fixtures";
import { WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY } from "../../state/workspace-provider-defaults";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  issueWorkItemPlanRecord,
  lifecycleCardTitle,
  lifecycleFetch,
  projectRecord,
  repositoryRecord,
  type LifecycleFetchMock,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

describe("IssueLifecycleWorkbench generation actions", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("shows spec version badges on lifecycle cards when generated content exists", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 6：单阶段面板——完整生命周期默认落在 work_item，先切到 Story 阶段。
    await user.click(await screen.findByTestId("stage-tab-story"));
    const storyColumn = screen.getByRole("region", {
      name: "Story Spec 内容",
    });

    expect(storyColumn).toHaveTextContent("v1");
  });

  it("shows generated spec markdown previews on lifecycle cards", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("stage-tab-story"));
    const storyColumn = screen.getByRole("region", {
      name: "Story Spec 内容",
    });
    expect(storyColumn).toHaveTextContent("[REQ-001] 显示会话过期提示");
  });

  it("generates story workspace from the issue card action and opens the story session", async () => {
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    // Task 7：队列改为 IssueQueue（行选择按钮名为「选择 Issue <标题>」，
    // 生成入口仍在同名 region 内，aria-label 保持「生成 Story Spec」）。
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });
    await user.click(
      within(
        screen.getByRole("region", { name: "Issue 卡片列表" }),
      ).getByRole("button", { name: "生成 Story Spec 登录会话过期" }),
    );

    expect(
      await screen.findByRole("button", { name: "登录会话过期 Story Spec" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_story_0001",
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "登录会话过期 Story Spec",
        }),
      }),
    );
  });

  it("does not expose the story generation action as a global header action", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ emptyLifecycle: true }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    const header = screen.getAllByRole("banner")[0];
    expect(
      within(header).queryByRole("button", { name: "生成 Story Spec 登录会话过期" }),
    ).not.toBeInTheDocument();
    // Task 6：除 Issue 卡入口外，空 story 阶段面板也常驻提供该动作。
    expect(
      within(
        screen.getByRole("region", { name: "Issue 卡片列表" }),
      ).getByRole("button", { name: "生成 Story Spec 登录会话过期" }),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByRole("region", { name: "Story Spec 内容" }),
      ).getByRole("button", { name: "生成 Story Spec" }),
    ).toBeInTheDocument();
  });

  it("prepares work item plan from design spec drawer and opens workspace", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    // Task 6：Design 卡片在 Design 阶段页内。
    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByRole("button", { name: "前端提示设计" }));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    await user.click(screen.getByTestId("stage-tab-work_item"));
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).toHaveTextContent("0 个 Work Item");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "前端提示设计 Work Item",
          story_spec_ids: ["story_spec_0001"],
          design_spec_ids: ["design_spec_0001"],
          include_integration_tests: true,
          include_e2e_tests: false,
          force_frontend_backend_split: true,
          require_execution_plan_confirm: false,
        }),
      }),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(
        /^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/,
      ),
      expect.anything(),
    );

    await user.click(
      screen.getByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    await user.click(
      within(workItemRegion).getByRole("button", { name: "Work Item Group" }),
    );
    expect(await screen.findByTestId("work-item-group-children")).toHaveTextContent(
      "暂无子 Work Item",
    );

    onOpenWorkspace.mockClear();
    await user.click(screen.getByTestId("drawer-open-workspace"));
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_plan_group_0001",
    );
  });

  it("generates next design spec from story spec drawer without opening workspace or running providers", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    // Task 6：Story 卡片在 Story 阶段页内。
    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    await user.click(screen.getByRole("button", { name: "生成 Design Spec" }));

    // Task 6：新生成的 Design 卡片属于 Design 阶段页，切过去断言它已入列。
    await user.click(await screen.findByTestId("stage-tab-design"));
    expect(
      await screen.findByRole("button", { name: "会话过期提示 Design Spec" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).not.toHaveBeenCalled();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "会话过期提示 Design Spec",
          story_spec_ids: ["story_spec_0001"],
        }),
      }),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(
        /^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/,
      ),
      expect.anything(),
    );
    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
        "design_spec:issue_0001:design_spec_0002",
      ),
    );
  });

  it("opens work item plan options before preparing plan from a confirmed design", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));

    expect(
      await screen.findByRole("dialog", { name: "Work Item Plan 配置" }),
    ).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(([url]) =>
        String(url).includes("/work-item-plans:prepare"),
      ),
    ).toBe(false);
  });

  it("sends default work item split options after confirming the dialog", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });

    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        expect.objectContaining({
          method: "POST",
          body: expect.stringContaining('"force_frontend_backend_split":true'),
        }),
      ),
    );
    const prepareCall = fetchMock.mock.calls.find(([url]) =>
      String(url).includes("/work-item-plans:prepare"),
    );
    expect(prepareCall).toBeDefined();
    const body = JSON.parse(prepareCall?.[1]?.body as string) as Record<
      string,
      unknown
    >;
    expect(body).toMatchObject({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    });
  });

  it("sends selected work item split options after confirming the dialog", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });

    await user.click(within(dialog).getByLabelText("包含 E2E 测试 Work Item"));
    await user.click(
      within(dialog).getByLabelText("子 Work Item 执行前需要确认 Plan"),
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    const prepareCall = fetchMock.mock.calls.find(([url]) =>
      String(url).includes("/work-item-plans:prepare"),
    );
    expect(prepareCall).toBeDefined();
    const body = JSON.parse(prepareCall?.[1]?.body as string) as Record<
      string,
      unknown
    >;
    expect(body).toMatchObject({
      include_integration_tests: true,
      include_e2e_tests: true,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: true,
    });
  });

  it("does not prepare a work item plan when options dialog is cancelled", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });

    await user.click(within(dialog).getByRole("button", { name: "取消" }));

    expect(
      screen.queryByRole("dialog", { name: "Work Item Plan 配置" }),
    ).not.toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(([url]) =>
        String(url).includes("/work-item-plans:prepare"),
      ),
    ).toBe(false);
  });
});

const PROVIDER_LABELS: Record<RealProviderName, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  pi: "Pi",
  kimi_code: "Kimi Code",
};

/** 可用性快照 fixture：只声明关心的 provider，其余按不可用构造（镜像 /api/providers/status）。 */
function setProviderAvailability(
  available: Partial<Record<RealProviderName, boolean>>,
) {
  useProviderAvailabilityStore.setState({
    loadStatus: "loaded",
    snapshot: providerHealthSnapshot(available),
  });
}

// REQ-PPS-01：创建请求携带 provider 快照——plan 弹窗内选择、story/design 自动补默认。
describe("IssueLifecycleWorkbench plan provider snapshots", () => {
  installIssueLifecycleWorkbenchTestHooks();

  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    useProviderAvailabilityStore.getState().reset();
  });

  function writeProviderDefaults(author: string, reviewer: string) {
    window.localStorage.setItem(
      WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY,
      JSON.stringify({ author, reviewer, reviewerEnabled: true }),
    );
  }

  async function openPlanDialog(user: UserEvent) {
    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    return await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });
  }

  function requestBody(
    fetchMock: LifecycleFetchMock,
    path: string,
  ): Record<string, unknown> {
    const call = fetchMock.mock.calls.find(([url]) =>
      String(url).includes(path),
    );
    expect(call).toBeDefined();
    return JSON.parse(String(call?.[1]?.body)) as Record<string, unknown>;
  }

  /** 把某个创建端点整体替换成 4xx 失败响应（provider 不可用等服务端 fail-closed）。 */
  function failRequests(
    baseFetch: LifecycleFetchMock,
    pathFragment: string,
    message: string,
  ): LifecycleFetchMock {
    return vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        if (String(input).includes(pathFragment)) {
          return Promise.resolve(
            new Response(
              JSON.stringify({ code: "provider_unavailable", message }),
              { status: 400, headers: { "content-type": "application/json" } },
            ),
          );
        }
        return baseFetch(input, init);
      },
    ) as LifecycleFetchMock;
  }

  it("prefills the plan provider selects from stored defaults and locks unavailable providers", async () => {
    const user = userEvent.setup();
    vi.stubGlobal("fetch", lifecycleFetch());
    writeProviderDefaults("codex", "claude_code");
    setProviderAvailability({ codex: true, claude_code: false, pi: true });

    render(<IssueLifecycleWorkbench />);

    const dialog = await openPlanDialog(user);
    const authorSelect = within(dialog).getByLabelText("Author Provider");
    const reviewerSelect = within(dialog).getByLabelText("Reviewer Provider");

    expect(authorSelect).toHaveValue("codex");
    expect(reviewerSelect).toHaveValue("claude_code");
    expect(
      within(authorSelect).getByRole("option", { name: "Claude Code" }),
    ).toBeDisabled();
    expect(
      within(reviewerSelect).getByRole("option", { name: "Codex" }),
    ).toBeEnabled();
  });

  it("sends the plan creation request with the providers chosen in the dialog", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();
    writeProviderDefaults("codex", "codex");
    setProviderAvailability({ codex: true, pi: true });

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    const dialog = await openPlanDialog(user);
    await user.selectOptions(
      within(dialog).getByLabelText("Author Provider"),
      "pi",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    expect(requestBody(fetchMock, "/work-item-plans:prepare")).toMatchObject({
      author_provider: "pi",
      reviewer_provider: "codex",
    });
  });

  it("omits provider fields when no user default is stored", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    setProviderAvailability({ codex: true });

    render(<IssueLifecycleWorkbench onOpenWorkspace={vi.fn()} />);

    const dialog = await openPlanDialog(user);
    expect(within(dialog).getByLabelText("Author Provider")).toHaveValue("");
    expect(within(dialog).getByLabelText("Reviewer Provider")).toHaveValue("");
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([url]) =>
          String(url).includes("/work-item-plans:prepare"),
        ),
      ).toBe(true),
    );
    const body = requestBody(fetchMock, "/work-item-plans:prepare");
    expect(body).not.toHaveProperty("author_provider");
    expect(body).not.toHaveProperty("reviewer_provider");
  });

  it("keeps the plan dialog open and shows the provider_unavailable message inline", async () => {
    const user = userEvent.setup();
    const fetchMock = failRequests(
      lifecycleFetch(),
      "/work-item-plans:prepare",
      "Provider Codex 当前不可用，请重新选择",
    );
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();
    writeProviderDefaults("codex", "codex");

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    const dialog = await openPlanDialog(user);
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Provider Codex 当前不可用，请重新选择",
    );
    expect(
      screen.getByRole("dialog", { name: "Work Item Plan 配置" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).not.toHaveBeenCalled();
  });

  // REQ-PPS-01 场景三：story/design 创建被服务端 fail-closed 拒绝时必须有可诊断反馈
  // ——这两条入口没有弹窗承载错误，落工作台错误横幅（不再是无处可去的 rejection）。
  it("surfaces a rejected story creation with the stored provider on the workbench banner", async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      failRequests(
        lifecycleFetch({ emptyLifecycle: true }),
        "/story-specs:generate",
        "Provider pi 当前不可用",
      ),
    );
    const onOpenWorkspace = vi.fn();
    writeProviderDefaults("pi", "kimi_code");

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });
    await user.click(
      within(screen.getByRole("region", { name: "Issue 卡片列表" })).getByRole(
        "button",
        { name: "生成 Story Spec 登录会话过期" },
      ),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Provider pi 当前不可用",
    );
    expect(onOpenWorkspace).not.toHaveBeenCalled();
  });

  it("surfaces a rejected next-design creation with the stored provider on the workbench banner", async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      failRequests(
        lifecycleFetch(),
        "/design-specs:generate",
        "Provider kimi_code 当前不可用",
      ),
    );
    writeProviderDefaults("pi", "kimi_code");

    render(<IssueLifecycleWorkbench onOpenWorkspace={vi.fn()} />);

    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    await user.click(screen.getByRole("button", { name: "生成 Design Spec" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Provider kimi_code 当前不可用",
    );
  });

  it("sends the stored provider defaults with the story generation request", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    writeProviderDefaults("pi", "kimi_code");

    render(<IssueLifecycleWorkbench onOpenWorkspace={vi.fn()} />);

    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });
    await user.click(
      within(screen.getByRole("region", { name: "Issue 卡片列表" })).getByRole(
        "button",
        { name: "生成 Story Spec 登录会话过期" },
      ),
    );

    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([url]) =>
          String(url).includes("/story-specs:generate"),
        ),
      ).toBe(true),
    );
    expect(requestBody(fetchMock, "/story-specs:generate")).toMatchObject({
      author_provider: "pi",
      reviewer_provider: "kimi_code",
    });
  });

  it("sends the stored provider defaults with the design generation request", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    writeProviderDefaults("pi", "kimi_code");

    render(<IssueLifecycleWorkbench onOpenWorkspace={vi.fn()} />);

    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    await user.click(screen.getByRole("button", { name: "生成 Design Spec" }));

    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([url]) =>
          String(url).includes("/design-specs:generate"),
        ),
      ).toBe(true),
    );
    expect(requestBody(fetchMock, "/design-specs:generate")).toMatchObject({
      author_provider: "pi",
      reviewer_provider: "kimi_code",
    });
  });
});
