import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  issueWorkItemPlanRecord,
  jsonResponseValue,
  lifecycleCardTitle,
  lifecycleFetch,
  projectRecord,
  projectsBody,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

describe("IssueLifecycleWorkbench project and lifecycle CRUD", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("creates repository from the left sidebar and enables issue creation", async () => {
    const fetchMock = lifecycleFetch({
      repositoriesByProject: { project_0001: [] },
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByText("还没有代码库");
    expect(screen.getByRole("button", { name: "新建 Issue" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "添加代码库" }));
    const dialog = screen.getByRole("dialog", { name: "添加代码库" });
    await user.type(within(dialog).getByLabelText("代码库名称"), "New Repo");
    await user.type(within(dialog).getByLabelText("本地路径"), "/tmp/new-repo");
    await user.selectOptions(
      within(dialog).getByLabelText("Policy"),
      "manual-all",
    );
    await user.selectOptions(
      within(dialog).getByLabelText("Provider"),
      "claude_code",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "添加代码库" }),
    );

    expect(await screen.findByText("New Repo")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "新建 Issue" })).toBeEnabled();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repositories",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          name: "New Repo",
          path: "/tmp/new-repo",
          default_policy_preset: "manual-all",
          default_provider_mode: "claude_code",
        }),
      }),
    );
  });

  it("deletes project repositories and lifecycle issues from the lifecycle workbench", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByRole("button", { name: "登录会话过期" }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "删除代码库 Aria Repo" }),
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repositories/repository_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有代码库")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "删除 Issue 登录会话过期" }),
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "登录会话过期" }),
      ).not.toBeInTheDocument(),
    );

    await user.click(screen.getByRole("button", { name: "删除 Project Aria" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
  });

  it("deletes specs and work item groups from selected issue content", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    await user.click(
      within(screen.getByRole("region", { name: "Story Spec 内容" })).getByRole(
        "button",
        { name: "删除 Story Spec 会话过期提示" },
      ),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Story Spec 内容" }),
      ).not.toHaveTextContent("会话过期提示"),
    );

    await user.click(
      within(
        screen.getByRole("region", { name: "Design Spec 内容" }),
      ).getByRole("button", { name: "删除 Design Spec 前端提示设计" }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/design-specs/design_spec_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Design Spec 内容" }),
      ).not.toHaveTextContent("前端提示设计"),
    );

    await user.click(
      within(screen.getByRole("region", { name: "Work Item 内容" })).getByRole(
        "button",
        { name: /删除 Work Item Group/u },
      ),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Work Item 内容" }),
      ).not.toHaveTextContent("Work Item Group"),
    );
  });

  it("requires repository when creating issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByRole("region", { name: "Issue 卡片列表" });
    await user.click(screen.getByRole("button", { name: "新建 Issue" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(
      within(dialog).getByLabelText("Issue 标题"),
      "新增安全提示",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Issue" }),
    );

    expect(within(dialog).getByText("请选择代码库")).toBeInTheDocument();
  });

  it("shows an alert for invalid lifecycle responses", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ invalidLifecycle: true }));

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "invalid lifecycle response",
    );
  });

  it("shows an alert for invalid work item plan detail fields", async () => {
    const { options: _options, ...missingOptions } = issueWorkItemPlanRecord();
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        workItemPlans: [
          missingOptions,
          {
            ...issueWorkItemPlanRecord({ id: "issue_plan_wrong_shape" }),
            work_item_ids: "work_item_0001",
          },
        ],
      }),
    );

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "invalid lifecycle response",
    );
    expect(
      screen.queryByRole("button", { name: "Work Item Group" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the latest refresh result when an older request finishes later", async () => {
    const firstProjects = deferred<Response>();
    const secondProjects = deferred<Response>();
    const fetchMock = lifecycleFetch({
      projectResponses: [firstProjects.promise, secondProjects.promise],
      issueTitles: ["最新 Issue", "旧 Issue"],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(screen.getByRole("button", { name: "刷新" }));

    secondProjects.resolve(jsonResponseValue(projectsBody()));
    expect(
      await screen.findByRole("button", { name: "最新 Issue" }),
    ).toBeInTheDocument();

    firstProjects.resolve(jsonResponseValue(projectsBody()));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(
      screen.queryByRole("button", { name: "旧 Issue" }),
    ).not.toBeInTheDocument();
  });

  it("does not mark derived cards selected when their id matches an issue id", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ duplicateCardIds: true }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "重复 ID Issue" }),
    );

    expect(
      screen.getByRole("button", { name: "重复 ID Issue" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: "重复 ID Story" }),
    ).toHaveAttribute("aria-pressed", "false");
  });

  it("opens drawer from derived lifecycle cards and opens full screen workspace from drawer CTA", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByRole("button", { name: "会话过期提示" });
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));

    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "会话过期提示",
    );
    expect(onOpenWorkspace).not.toHaveBeenCalled();

    await user.click(screen.getByTestId("drawer-open-workspace"));
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_story_0001",
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(
        /^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/,
      ),
      expect.anything(),
    );
  });

  it("selects confirmed story cards so downstream design generation is reachable", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(
      await screen.findByRole("button", { name: "会话过期提示" }),
    );

    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "会话过期提示",
    );
    expect(onOpenWorkspace).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "生成 Design Spec" }),
    ).toBeInTheDocument();
  });
});
