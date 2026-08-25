import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import { defaultCollapsedGroups } from "./issue-queue-derivation";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  jsonResponseValue,
  lifecycleFetch,
  projectRecord,
  projectsBody,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-utils";

// Task 7：批量 Issue fixture（全部无 Story -> 单一 needs_story 组），用于验证
// 「显示更多」跨越 deriveIssueQueue 默认 perGroupLimit（50）后真正追加渲染。
function bulkIssuesFetch(titles: string[]) {
  const issues = titles.map((title, index) => ({
    issue_id: `issue_${String(index + 1).padStart(4, "0")}`,
    project_id: "project_0001",
    repo_id: "repository_0001",
    workspace_id: null,
    task_id: null,
    session_id: null,
    title,
    description: "描述",
    change_id: null,
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  }));

  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/projects") {
      return jsonResponse({ projects: [projectRecord("project_0001", "Aria")] });
    }
    if (url === "/api/projects/project_0001/repositories") {
      return jsonResponse({ repositories: [repositoryRecord()] });
    }
    if (url === "/api/projects/project_0001/codebases") {
      return jsonResponse({ codebases: [] });
    }
    if (url === "/api/projects/project_0001/issues") {
      return jsonResponse({ issues });
    }
    const lifecycleMatch = url.match(
      /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/,
    );
    if (lifecycleMatch) {
      const issue = issues.find(
        (candidate) => candidate.issue_id === lifecycleMatch[1],
      );
      return jsonResponse({
        issue,
        story_specs: [],
        design_specs: [],
        work_item_plans: [],
        work_items: [],
        work_item_repository_groups: [],
        workspace_sessions: [],
        coding_attempts: [],
      });
    }
    return jsonResponse({});
  });
}

describe("IssueLifecycleWorkbench 队列折叠双密度 (Task 7)", () => {
  installIssueLifecycleWorkbenchTestHooks();

  function queueRegion() {
    return screen.getByRole("region", { name: "Issue 卡片列表" });
  }

  // 折叠按钮在队列列内、region 之外（IssueQueue 契约不含折叠控件）。
  function queueColumn() {
    return screen.getByTestId("issue-queue-column");
  }

  it("外壳与队列列宽满足双密度规格，队列与工作区各自可滚动", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    expect(screen.getByTestId("workbench-shell")).toHaveClass("h-[100dvh]");
    const column = screen.getByTestId("issue-queue-column");
    expect(column).toHaveClass("w-72", "shrink-0");
    expect(screen.getByTestId("issue-queue-group-list")).toHaveClass(
      "min-h-0",
      "overflow-y-auto",
    );
  });

  it("折叠队列显示细轨（含展开按钮与计数）且工作区仍在，展开后恢复队列", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );

    const rail = screen.getByTestId("issue-queue-collapsed-rail");
    expect(rail).toHaveClass("w-10", "shrink-0");
    expect(
      within(rail).getByRole("button", { name: "展开 Issue 队列" }),
    ).toBeInTheDocument();
    expect(within(rail).getByTestId("issue-queue-rail-count")).toHaveTextContent(
      "1",
    );
    expect(
      screen.queryByRole("region", { name: "Issue 卡片列表" }),
    ).not.toBeInTheDocument();
    // 工作区仍在（专注密度只切换队列，不影响工作区）。
    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("stage-stepper")).toBeInTheDocument();

    await user.click(
      within(rail).getByRole("button", { name: "展开 Issue 队列" }),
    );

    expect(
      screen.getByRole("region", { name: "Issue 卡片列表" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("issue-queue-collapsed-rail")).toBeNull();
  });

  it("折叠状态按 projectId 写入 localStorage 并在重挂载后恢复", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    const view = render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );
    expect(
      window.localStorage.getItem("aria.workbench.queueCollapsed.project_0001"),
    ).toBe("1");

    view.unmount();
    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByTestId("issue-queue-collapsed-rail"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "展开 Issue 队列" }),
    );
    expect(
      window.localStorage.getItem("aria.workbench.queueCollapsed.project_0001"),
    ).toBe("0");
  });

  it("折叠状态按 Project 相互独立", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [
          projectRecord("project_0001", "Aria"),
          projectRecord("project_0002", "Mobile"),
        ],
        issueTitlesByProject: {
          project_0001: "登录会话过期",
          project_0002: "移动端刷新",
        },
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );
    expect(screen.getByTestId("issue-queue-collapsed-rail")).toBeInTheDocument();

    // 切到 project_0002：该 Project 未折叠，队列可见。
    await user.click(screen.getByRole("button", { name: "Mobile" }));
    await screen.findByRole("button", { name: "选择 Issue 移动端刷新" });
    expect(screen.queryByTestId("issue-queue-collapsed-rail")).toBeNull();

    // 切回 project_0001：恢复其折叠态。
    await user.click(screen.getByRole("button", { name: "Aria" }));
    expect(
      await screen.findByTestId("issue-queue-collapsed-rail"),
    ).toBeInTheDocument();
    expect(
      window.localStorage.getItem("aria.workbench.queueCollapsed.project_0002"),
    ).not.toBe("1");
  });

  it("折叠与展开不改变聚焦 Issue、不改变选中实体、不触发网络请求", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    await user.click(screen.getByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
        "story_spec:issue_0001:story_spec_0001",
      ),
    );
    const callsBefore = fetchMock.mock.calls.length;

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );
    await user.click(
      screen.getByRole("button", { name: "展开 Issue 队列" }),
    );

    expect(fetchMock.mock.calls).toHaveLength(callsBefore);
    // 聚焦 Issue 未变：其行仍高亮（aria-current）。
    const focusedRow = screen
      .getAllByTestId("issue-queue-row")
      .find((row) => row.getAttribute("data-issue-id") === "issue_0001");
    expect(focusedRow).toHaveAttribute("aria-current", "true");
    // 选中实体未变：drawer 仍聚焦同一 Story Spec。
    expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
      "story_spec:issue_0001:story_spec_0001",
    );
  });

  it("分组折叠默认为 defaultCollapsedGroups 并按 projectId 持久化", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    // 缺省折叠组 = defaultCollapsedGroups()（["completed"]）。
    for (const key of defaultCollapsedGroups()) {
      const header = screen
        .queryAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === key);
      if (header) {
        expect(header).toHaveAttribute("aria-expanded", "false");
      }
    }

    // fixture 的唯一 Issue 落在 needs_work_item 组（story/design 有产物、无 work item plan 时）
    // 或 coding/blocked 组；取当前渲染出的第一个组头折叠它并断言持久化。
    const header = screen.getAllByTestId("issue-queue-group-header")[0];
    const groupKey = header.getAttribute("data-group-key");
    expect(groupKey).not.toBeNull();
    expect(header).toHaveAttribute("aria-expanded", "true");
    await user.click(header);

    expect(
      screen
        .getAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === groupKey),
    ).toHaveAttribute("aria-expanded", "false");
    const stored = window.localStorage.getItem(
      "aria.workbench.groups.project_0001",
    );
    expect(stored).not.toBeNull();
    expect(JSON.parse(String(stored))).toEqual(
      expect.arrayContaining([...defaultCollapsedGroups(), groupKey]),
    );
  });

  it("队列过滤命中时保留 Issue，未命中时显示空态", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.type(screen.getByLabelText("过滤 Issues"), "登录");
    expect(
      screen.getByRole("button", { name: "选择 Issue 登录会话过期" }),
    ).toBeInTheDocument();

    await user.clear(screen.getByLabelText("过滤 Issues"));
    await user.type(screen.getByLabelText("过滤 Issues"), "不存在的关键字");
    expect(
      screen.queryByRole("button", { name: "选择 Issue 登录会话过期" }),
    ).toBeNull();
    expect(queueRegion()).toHaveTextContent("没有匹配的 Issue。");
  });

  it("「显示更多」真正追加渲染该组行，追加后入口消失", async () => {
    // 构造 60 个 Issue（> DEFAULT_PER_GROUP_LIMIT=50）使组内 rows 被截断。
    const titles = Array.from(
      { length: 60 },
      (_, index) => `批量 Issue ${String(index + 1).padStart(2, "0")}`,
    );
    vi.stubGlobal("fetch", bulkIssuesFetch(titles));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByTestId("issue-queue-group-list");

    await waitFor(() =>
      expect(screen.getAllByTestId("issue-queue-row")).toHaveLength(50),
    );
    const showMore = screen.getByTestId("issue-queue-show-more");
    expect(showMore).toHaveTextContent("显示更多（+10）");

    await user.click(showMore);

    await waitFor(() =>
      expect(screen.getAllByTestId("issue-queue-row")).toHaveLength(60),
    );
    expect(screen.queryByTestId("issue-queue-show-more")).toBeNull();
  });

  it("refresh 不重置队列折叠态、过滤文本与聚焦 Issue（轮询上下文冻结）", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    // 折叠一个分组 + 输入过滤文本（仍命中当前 Issue）。
    const header = screen.getAllByTestId("issue-queue-group-header")[0];
    const groupKey = header.getAttribute("data-group-key");
    await user.click(header);
    await user.type(screen.getByLabelText("过滤 Issues"), "登录");

    // 触发一次 refresh（与 2s 轮询同一条 refresh() 链路）。
    const callsBefore = fetchMock.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() =>
      expect(fetchMock.mock.calls.length).toBeGreaterThan(callsBefore),
    );
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());

    // 分组折叠态、过滤文本、聚焦 Issue 全部保持。
    expect(
      screen
        .getAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === groupKey),
    ).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByLabelText("过滤 Issues")).toHaveValue("登录");
    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toHaveTextContent("登录会话过期");
  });

  it("refresh（deferred）冻结聚焦 Issue、抽屉、分组折叠与过滤文本", async () => {
    const firstProjects = deferred<Response>();
    const secondProjects = deferred<Response>();
    const baseFetch = lifecycleFetch({
      projectResponses: [firstProjects.promise, secondProjects.promise],
    });
    // 双 Issue fixture：issue_0001（coding 组，完整生命周期）与 issue_0002
    // （needs_story 组，空生命周期）。这样可同时断言「被折叠的分组」与「聚焦
    // Issue 行」（若折叠唯一分组会连聚焦行一起隐藏，无法验证 aria-current 保留）。
    const issueOne = {
      issue_id: "issue_0001",
      project_id: "project_0001",
      repo_id: "repository_0001",
      workspace_id: null,
      task_id: null,
      session_id: null,
      title: "登录会话过期",
      description: "描述",
      change_id: null,
      phase: "clarification",
      status: "draft",
      active_binding_id: null,
      artifacts: [],
      created_at: "2026-05-16T00:00:00Z",
      updated_at: "2026-05-16T00:00:00Z",
    };
    const issueTwo = { ...issueOne, issue_id: "issue_0002", title: "缺少 Story" };
    const fetchMock = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url === "/api/projects/project_0001/issues") {
          return jsonResponse({ issues: [issueOne, issueTwo] });
        }
        if (url === "/api/issues/issue_0002/lifecycle?project_id=project_0001") {
          return jsonResponse({
            issue: issueTwo,
            story_specs: [],
            design_specs: [],
            work_item_plans: [],
            work_items: [],
            work_item_repository_groups: [],
            workspace_sessions: [],
            coding_attempts: [],
          });
        }
        return baseFetch(input, init);
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // 首次加载：放行 /api/projects（与后续 refresh 返回相同数据）。
    await act(async () => {
      firstProjects.resolve(jsonResponseValue(projectsBody()));
    });

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    expect(
      screen
        .getAllByTestId("issue-queue-row")
        .find((row) => row.getAttribute("data-issue-id") === "issue_0001"),
    ).toHaveAttribute("aria-current", "true");

    // 折叠不含聚焦 Issue 的分组（needs_story）+ 输入过滤文本（仍命中两个 Issue）。
    const header = screen.getAllByTestId("issue-queue-group-header")[0];
    const groupKey = header.getAttribute("data-group-key");
    await user.click(header);
    await user.type(screen.getByLabelText("过滤 Issues"), "issue");

    // Task 6：Story 卡在 Story 阶段页内；切到 Story 阶段后点击打开抽屉。
    await user.click(screen.getByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    expect(screen.getByTestId("lifecycle-card-drawer")).toBeInTheDocument();

    // 触发一次 refresh（与 2s 轮询同一条 refresh() 链路），用 deferred
    // 控制 /api/projects 返回相同数据。
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await act(async () => {
      secondProjects.resolve(jsonResponseValue(projectsBody()));
    });
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());

    // 上下文冻结：聚焦 Issue（aria-current）、抽屉、分组折叠、过滤文本全部保持。
    expect(
      screen
        .getAllByTestId("issue-queue-row")
        .find((row) => row.getAttribute("data-issue-id") === "issue_0001"),
    ).toHaveAttribute("aria-current", "true");
    expect(screen.getByTestId("lifecycle-card-drawer")).toBeInTheDocument();
    expect(
      screen
        .getAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === groupKey),
    ).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByLabelText("过滤 Issues")).toHaveValue("issue");
  });

  it("队列行动作复用既有 handler：选择、生成 Story Spec、删除 Issue", async () => {
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toHaveTextContent("登录会话过期");

    await user.click(
      within(queueRegion()).getByRole("button", { name: "生成 Story Spec 登录会话过期" }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
      expect.objectContaining({ method: "POST" }),
    );

    await user.click(
      within(queueRegion()).getByRole("button", {
        name: "删除 Issue 登录会话过期",
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
  });
}
);