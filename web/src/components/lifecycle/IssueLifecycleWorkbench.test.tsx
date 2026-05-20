import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceSession } from "../../api/types";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";

type MockLifecycleData = {
  story_specs: Array<Record<string, unknown>>;
  design_specs: Array<Record<string, unknown>>;
  work_items: Array<Record<string, unknown>>;
  workspace_sessions: WorkspaceSession[];
};

describe("IssueLifecycleWorkbench", () => {
  beforeEach(() => {
    useLifecycleWorkbenchStore.setState({
      focusedEntityId: null,
      isDrawerOpen: false,
    });
  });

  it("renders four lifecycle columns and focuses derived cards by issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("navigation", { name: "Project 切换" })).toHaveTextContent(
      "Aria",
    );
    expect(await screen.findByRole("region", { name: "Issue 列" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Story Spec 列" })).toHaveTextContent(
      "会话过期提示",
    );
    expect(screen.getByRole("region", { name: "Design Spec 列" })).toHaveTextContent(
      "前端提示设计",
    );
    expect(screen.getByRole("region", { name: "Work Item 列" })).toHaveTextContent(
      "实现提示组件",
    );

    await user.click(screen.getByRole("button", { name: "登录会话过期" }));

    expect(screen.getByRole("button", { name: "显示全部" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Story Spec 列" })).toHaveTextContent(
      "会话过期提示",
    );
    expect(screen.getByRole("region", { name: "Design Spec 列" })).toHaveTextContent(
      "前端提示设计",
    );
    expect(screen.getByRole("region", { name: "Work Item 列" })).toHaveTextContent(
      "实现提示组件",
    );
  });

  it("syncs controlled URL focus with drawer state", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const onDrawerFocusChange = vi.fn();

    const view = render(
      <IssueLifecycleWorkbench
        focusEntityId="story_spec_0001"
        onDrawerFocusChange={onDrawerFocusChange}
      />,
    );

    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBe("story_spec_0001"),
    );
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(true);
    await waitFor(() => expect(onDrawerFocusChange).toHaveBeenCalledWith("story_spec_0001"));

    view.rerender(
      <IssueLifecycleWorkbench focusEntityId={null} onDrawerFocusChange={onDrawerFocusChange} />,
    );

    await waitFor(() => expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBeNull());
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(false);

    act(() => {
      useLifecycleWorkbenchStore.getState().openDrawer("design_spec_0001");
    });

    await waitFor(() => expect(onDrawerFocusChange).toHaveBeenCalledWith("design_spec_0001"));
  });

  it("switches project from the left sidebar", async () => {
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria"), projectRecord("project_0002", "Mobile")],
      issueTitlesByProject: {
        project_0001: "登录会话过期",
        project_0002: "移动端刷新",
      },
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("button", { name: "登录会话过期" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Mobile" }));

    expect(await screen.findByRole("button", { name: "移动端刷新" })).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0002/issues",
      expect.objectContaining({
        headers: expect.objectContaining({ "content-type": "application/json" }),
      }),
    );
  });

  it("creates project from the left sidebar and selects it", async () => {
    const fetchMock = lifecycleFetch({ projects: [] });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "新建 Project" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Project" });
    await user.type(within(dialog).getByLabelText("Project 名称"), "New Project");
    await user.type(within(dialog).getByLabelText("Project 描述"), "新的生命周期项目");
    await user.click(within(dialog).getByRole("button", { name: "创建 Project" }));

    expect(await screen.findByRole("button", { name: "New Project" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ name: "New Project", description: "新的生命周期项目" }),
      }),
    );
  });

  it("shows project repositories in the left sidebar", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);

    const sidebar = await screen.findByRole("navigation", { name: "Project 切换" });
    expect(sidebar).toHaveTextContent("Aria Repo");
    expect(sidebar).toHaveTextContent("/tmp/aria");
  });

  it("shows only repositories for the selected project", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria"), projectRecord("project_0002", "Mobile")],
        repositoriesByProject: {
          project_0001: [
            repositoryRecord({
              repository_id: "repository_0001",
              project_id: "project_0001",
              name: "Aria Repo",
              path: "/tmp/aria",
            }),
          ],
          project_0002: [
            repositoryRecord({
              repository_id: "repository_0002",
              project_id: "project_0002",
              name: "Mobile Repo",
              path: "/tmp/mobile",
            }),
          ],
        },
        issueTitlesByProject: {
          project_0001: "登录会话过期",
          project_0002: "",
        },
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const sidebar = await screen.findByRole("navigation", { name: "Project 切换" });
    expect(sidebar).toHaveTextContent("Aria Repo");
    expect(sidebar).not.toHaveTextContent("Mobile Repo");

    await user.click(screen.getByRole("button", { name: "Mobile" }));

    expect(await screen.findByText("Mobile Repo")).toBeInTheDocument();
    expect(sidebar).not.toHaveTextContent("Aria Repo");
  });

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
    await user.selectOptions(within(dialog).getByLabelText("Policy"), "manual-all");
    expect(within(dialog).queryByLabelText("Provider")).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "添加代码库" }));

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
        }),
      }),
    );
  });

  it("deletes project repositories and lifecycle issues from the lifecycle workbench", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("button", { name: "登录会话过期" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "删除代码库 Aria Repo" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repositories/repository_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有代码库")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "删除 Issue 登录会话过期" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "登录会话过期" })).not.toBeInTheDocument(),
    );

    await user.click(screen.getByRole("button", { name: "删除 Project Aria" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
  });

  it("requires repository when creating issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByRole("region", { name: "Issue 列" });
    await user.click(screen.getByRole("button", { name: "新建 Issue" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(within(dialog).getByLabelText("Issue 标题"), "新增安全提示");
    await user.click(within(dialog).getByRole("button", { name: "创建 Issue" }));

    expect(within(dialog).getByText("请选择代码库")).toBeInTheDocument();
  });

  it("shows an alert for invalid lifecycle responses", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ invalidLifecycle: true }));

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("alert")).toHaveTextContent("invalid lifecycle response");
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
    expect(await screen.findByRole("button", { name: "最新 Issue" })).toBeInTheDocument();

    firstProjects.resolve(jsonResponseValue(projectsBody()));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "旧 Issue" })).not.toBeInTheDocument();
  });

  it("does not mark derived cards selected when their id matches an issue id", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ duplicateCardIds: true }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "重复 ID Issue" }));

    expect(screen.getByRole("button", { name: "重复 ID Issue" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "重复 ID Story" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("opens full screen workspace sessions from derived lifecycle cards", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByRole("button", { name: "会话过期提示" });
    await user.click(screen.getByRole("button", { name: "打开 Workspace 会话过期提示" }));

    expect(onOpenWorkspace).toHaveBeenCalledWith("workspace_session_story_0001");

    await user.click(screen.getByRole("button", { name: "打开 Workspace 实现提示组件" }));

    expect(onOpenWorkspace).toHaveBeenLastCalledWith("workspace_session_work_item_0001");
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(/^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/),
      expect.anything(),
    );
  });

  it("selects confirmed story cards so downstream design generation is reachable", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByRole("button", { name: "会话过期提示" }));

    expect(onOpenWorkspace).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "生成 Design Spec" })).toBeInTheDocument();
  });

  it("shows spec version badges on lifecycle cards when generated content exists", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);

    const storyColumn = await screen.findByRole("region", { name: "Story Spec 列" });

    expect(storyColumn).toHaveTextContent("v1");
  });

  it("shows generated spec markdown previews on lifecycle cards", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);

    const storyColumn = await screen.findByRole("region", { name: "Story Spec 列" });
    expect(storyColumn).toHaveTextContent("[REQ-001] 显示会话过期提示");
  });

  it("generates story design and work item workspaces then opens full screen sessions", async () => {
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByRole("button", { name: "登录会话过期" }));
    await user.click(screen.getByRole("button", { name: "生成 Story Spec" }));

    expect(
      await screen.findByRole("button", { name: "登录会话过期 Story Spec" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).toHaveBeenCalledWith("workspace_session_story_0001");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "登录会话过期 Story Spec",
        }),
      }),
    );

    await user.click(screen.getByRole("button", { name: "生成 Design Spec" }));

    expect(
      await screen.findByRole("button", { name: "登录会话过期 Story Spec Design Spec" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).toHaveBeenLastCalledWith("workspace_session_design_0001");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "登录会话过期 Story Spec Design Spec",
          story_spec_ids: ["story_spec_0001"],
          design_kind: "frontend",
        }),
      }),
    );

    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));

    expect(
      await screen.findByRole("button", {
        name: "登录会话过期 Story Spec Design Spec Work Item",
      }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).toHaveBeenLastCalledWith("workspace_session_work_item_0001");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-items:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "登录会话过期 Story Spec Design Spec Work Item",
          story_spec_ids: ["story_spec_0001"],
          design_spec_ids: ["design_spec_0001"],
        }),
      }),
    );
  });
});

describe("CreateLifecycleIssueDialog", () => {
  it("shows submit errors and prevents duplicate submissions while pending", async () => {
    const submit = deferred<void>();
    const onCreate = vi.fn(() => submit.promise);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        repositories={[repositoryRecord()]}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "新增安全提示");
    await user.selectOptions(screen.getByLabelText("代码库"), "repository_0001");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledTimes(1);

    submit.reject(new Error("create issue failed"));
    expect(await screen.findByText("create issue failed")).toBeInTheDocument();
  });
});

function lifecycleFetch(options?: {
  duplicateCardIds?: boolean;
  emptyLifecycle?: boolean;
  invalidLifecycle?: boolean;
  issueTitles?: string[];
  issueTitlesByProject?: Record<string, string>;
  projects?: Array<ReturnType<typeof projectRecord>>;
  repositoriesByProject?: Record<string, ReturnType<typeof repositoryRecord>[]>;
  projectResponses?: Array<Promise<Response>>;
}) {
  const projects = [...(options?.projects ?? [projectRecord("project_0001", "Aria")])];
  const repositoriesByProject = new Map<string, ReturnType<typeof repositoryRecord>[]>(
    Object.entries(options?.repositoriesByProject ?? { project_0001: [repositoryRecord()] }),
  );
  const deletedIssueIdsByProject = new Map<string, Set<string>>();
  let projectCall = 0;
  const issueCallsByProject = new Map<string, number>();
  const latestIssueTitlesByProject = new Map<string, string>();
  const lifecycleByIssue = new Map<string, MockLifecycleData>();

  function lifecycleData(issueId: string) {
    const existing = lifecycleByIssue.get(issueId);
    if (existing) {
      return existing;
    }
    const initial = initialLifecycleData(issueId, options?.duplicateCardIds, options?.emptyLifecycle);
    lifecycleByIssue.set(issueId, initial);
    return initial;
  }

  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/projects" && init?.method === "POST") {
      const payload = JSON.parse(String(init.body)) as { name: string; description?: string | null };
      const project = projectRecord(
        `project_${String(projects.length + 1).padStart(4, "0")}`,
        payload.name,
        payload.description ?? null,
      );
      projects.push(project);
      return jsonResponse(project);
    }
    const projectDeleteMatch = url.match(/^\/api\/projects\/([^/]+)$/);
    if (projectDeleteMatch && init?.method === "DELETE") {
      const projectId = projectDeleteMatch[1];
      const index = projects.findIndex((project) => project.project_id === projectId);
      if (index >= 0) {
        projects.splice(index, 1);
      }
      repositoriesByProject.delete(projectId);
      deletedIssueIdsByProject.delete(projectId);
      return jsonResponse({ status: "deleted" });
    }
    if (url === "/api/projects") {
      const response = options?.projectResponses?.[projectCall];
      projectCall += 1;
      return response ?? jsonResponse({ projects });
    }
    const repositoryDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/repositories\/([^/]+)$/,
    );
    if (repositoryDeleteMatch && init?.method === "DELETE") {
      const projectId = repositoryDeleteMatch[1];
      const repositoryId = repositoryDeleteMatch[2];
      repositoriesByProject.set(
        projectId,
        (repositoriesByProject.get(projectId) ?? []).filter(
          (repository) => repository.repository_id !== repositoryId,
        ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const repositoryMatch = url.match(/^\/api\/projects\/([^/]+)\/repositories$/);
    if (repositoryMatch) {
      const projectId = repositoryMatch[1];
      if (init?.method === "POST") {
        const payload = JSON.parse(String(init.body)) as {
          name: string;
          path: string;
          default_policy_preset?: string | null;
          default_provider_mode?: string | null;
        };
        const repositories = repositoriesByProject.get(projectId) ?? [];
        const repository = repositoryRecord({
          repository_id: `repository_${String(repositories.length + 1).padStart(4, "0")}`,
          project_id: projectId,
          name: payload.name,
          path: payload.path,
          default_policy_preset: payload.default_policy_preset ?? "manual-write",
          default_provider_mode: payload.default_provider_mode ?? "fake",
        });
        repositoriesByProject.set(projectId, [...repositories, repository]);
        return jsonResponse(repository);
      }
      return jsonResponse({
        repositories: repositoriesByProject.get(projectId) ?? [],
      });
    }
    if (url === "/api/projects/project_0001/issues" && init?.method === "POST") {
      return jsonResponse({
        issue_id: "issue_0002",
        project_id: "project_0001",
        repo_id: "repository_0001",
        workspace_id: null,
        task_id: null,
        session_id: null,
        title: "新增安全提示",
        description: null,
        change_id: "new-security-hint",
        phase: "clarification",
        status: "draft",
        active_binding_id: null,
        artifacts: [],
        created_at: "2026-05-16T00:00:00Z",
        updated_at: "2026-05-16T00:00:00Z",
      });
    }
    const issueDeleteMatch = url.match(/^\/api\/projects\/([^/]+)\/issues\/([^/]+)$/);
    if (issueDeleteMatch && init?.method === "DELETE") {
      const projectId = issueDeleteMatch[1];
      const issueId = issueDeleteMatch[2];
      const deletedIssueIds = deletedIssueIdsByProject.get(projectId) ?? new Set<string>();
      deletedIssueIds.add(issueId);
      deletedIssueIdsByProject.set(projectId, deletedIssueIds);
      lifecycleByIssue.delete(issueId);
      return jsonResponse({ status: "deleted" });
    }
    const workspaceRunNextMatch = url.match(/^\/api\/workspace-sessions\/([^/]+)\/run-next$/);
    if (workspaceRunNextMatch) {
      const payload = JSON.parse(String(init?.body ?? "{}")) as { user_prompt?: string };
      const session = findSession(lifecycleByIssue, workspaceRunNextMatch[1]);
      if (session) {
        session.status = "waiting_for_human";
        session.messages = [
          ...session.messages,
          {
            role: "user",
            content: payload.user_prompt ?? "",
            created_at: "2026-05-16T00:00:00Z",
          },
          {
            role: "provider",
            content: "provider result",
            created_at: "2026-05-16T00:00:01Z",
          },
          {
            role: "reviewer",
            content: "reviewer result",
            created_at: "2026-05-16T00:00:02Z",
          },
        ];
      }
      return jsonResponse(session ?? {});
    }
    if (url === "/api/workspace-sessions/workspace_session_story_0001/message") {
      return jsonResponse({
        ...workspaceSessionRecord("story", "story_spec_0001", "workspace_session_story_0001"),
        messages: [
          {
            role: "user",
            content: "请补充验收标准",
            created_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    const workspaceMessageMatch = url.match(/^\/api\/workspace-sessions\/([^/]+)\/message$/);
    if (workspaceMessageMatch) {
      const session = findSession(lifecycleByIssue, workspaceMessageMatch[1]);
      return jsonResponse(session ?? {});
    }
    const workspaceConfirmMatch = url.match(/^\/api\/workspace-sessions\/([^/]+)\/confirm$/);
    if (workspaceConfirmMatch) {
      const session = findSession(lifecycleByIssue, workspaceConfirmMatch[1]);
      if (session) {
        session.status = "confirmed";
      }
      const story = findStoryBySession(lifecycleByIssue, workspaceConfirmMatch[1]);
      if (story) {
        story.confirmation_status = "confirmed";
      }
      const design = findDesignBySession(lifecycleByIssue, workspaceConfirmMatch[1]);
      if (design) {
        design.confirmation_status = "confirmed";
      }
      return jsonResponse(session ?? {});
    }
    const storyGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/story-specs:generate$/,
    );
    if (storyGenerateMatch) {
      const issueId = storyGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const story = {
        story_spec_id: "story_spec_0001",
        issue_id: issueId,
        repository_id: "repository_0001",
        title: payload.title,
        current_version: null,
        current_markdown_preview: null,
        confirmation_status: "confirmed",
        artifact_versions: [],
      };
      const session = workspaceSessionRecord("story", "story_spec_0001", "workspace_session_story_0001", {
        author_provider: payload.author_provider,
        reviewer_provider: payload.reviewer_provider,
        review_rounds: payload.review_rounds,
        superpowers_enabled: payload.superpowers_enabled,
        openspec_enabled: payload.openspec_enabled,
        status: "open",
      });
      lifecycle.story_specs = [story];
      lifecycle.workspace_sessions = [session];
      return jsonResponse({ story_specs: [story], workspace_session: session });
    }
    const designGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/design-specs:generate$/,
    );
    if (designGenerateMatch) {
      const issueId = designGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        story_spec_ids: string[];
        design_kind: "frontend" | "backend";
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const design = {
        design_spec_id: "design_spec_0001",
        issue_id: issueId,
        story_spec_ids: payload.story_spec_ids,
        design_kind: payload.design_kind,
        title: payload.title,
        current_version: null,
        current_markdown_preview: null,
        confirmation_status: "confirmed",
        artifact_versions: [],
      };
      const session = workspaceSessionRecord(
        "design",
        "design_spec_0001",
        "workspace_session_design_0001",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.design_specs = [design];
      lifecycle.workspace_sessions.push(session);
      return jsonResponse({ design_specs: [design], workspace_session: session });
    }
    const workItemsGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-items:generate$/,
    );
    if (workItemsGenerateMatch) {
      const issueId = workItemsGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        story_spec_ids: string[];
        design_spec_ids: string[];
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const workItem = {
        work_item_id: "work_item_0001",
        issue_id: issueId,
        repository_id: "repository_0001",
        story_spec_ids: payload.story_spec_ids,
        design_spec_ids: payload.design_spec_ids,
        title: payload.title,
        plan_status: "not_started",
        execution_status: "pending",
      };
      const session = workspaceSessionRecord(
        "work_item",
        "work_item_0001",
        "workspace_session_work_item_0001",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.work_items = [workItem];
      lifecycle.workspace_sessions.push(session);
      return jsonResponse({ work_items: [workItem], workspace_session: session });
    }
    const issuesMatch = url.match(/^\/api\/projects\/([^/]+)\/issues$/);
    if (issuesMatch) {
      const projectId = issuesMatch[1];
      const issueCall = issueCallsByProject.get(projectId) ?? 0;
      const title =
        options?.issueTitlesByProject?.[projectId] ??
        options?.issueTitles?.[issueCall] ??
        "登录会话过期";
      latestIssueTitlesByProject.set(projectId, title);
      issueCallsByProject.set(projectId, issueCall + 1);
      const issueId = options?.duplicateCardIds ? "shared_id" : "issue_0001";
      if (deletedIssueIdsByProject.get(projectId)?.has(issueId)) {
        return jsonResponse({ issues: [] });
      }
      if (projectId !== "project_0001") {
        if (deletedIssueIdsByProject.get(projectId)?.has("issue_0002")) {
          return jsonResponse({ issues: [] });
        }
        return jsonResponse({
          issues: title
            ? [
                {
                  issue_id: "issue_0002",
                  project_id: projectId,
                  repo_id: null,
                  workspace_id: null,
                  task_id: null,
                  session_id: null,
                  title,
                  description: "描述",
                  change_id: "mobile-refresh",
                  phase: "clarification",
                  status: "draft",
                  active_binding_id: null,
                  artifacts: [],
                  created_at: "2026-05-16T00:00:00Z",
                  updated_at: "2026-05-16T00:00:00Z",
                },
              ]
            : [],
        });
      }
      return jsonResponse({
        issues: [
          {
            issue_id: issueId,
            project_id: "project_0001",
            repo_id: "repository_0001",
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: options?.duplicateCardIds ? "重复 ID Issue" : title,
            description: "描述",
            change_id: "login-session-expired",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    const lifecycleMatch = url.match(/^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/);
    if (lifecycleMatch) {
      if (options?.invalidLifecycle) {
        return jsonResponse({});
      }
      const duplicate = options?.duplicateCardIds ?? false;
      const requestIssueId = lifecycleMatch[1];
      const projectId = lifecycleMatch[2];
      const issueId = duplicate ? "shared_id" : requestIssueId;
      const issueTitle = duplicate
        ? "重复 ID Issue"
        : (latestIssueTitlesByProject.get(projectId) ??
          options?.issueTitlesByProject?.[projectId] ??
          "登录会话过期");
      if (projectId !== "project_0001") {
        return jsonResponse({
          issue: {
            issue_id: issueId,
            project_id: projectId,
            repo_id: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: issueTitle,
            description: "描述",
            change_id: "mobile-refresh",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
          story_specs: [],
          design_specs: [],
          work_items: [],
          workspace_sessions: [],
        });
      }
      const data = lifecycleData(issueId);
      return jsonResponse({
        issue: {
          issue_id: issueId,
          project_id: "project_0001",
          repo_id: "repository_0001",
          workspace_id: null,
          task_id: null,
          session_id: null,
          title: issueTitle,
          description: "描述",
          change_id: "login-session-expired",
          phase: "clarification",
          status: "draft",
          active_binding_id: null,
          artifacts: [],
          created_at: "2026-05-16T00:00:00Z",
          updated_at: "2026-05-16T00:00:00Z",
        },
        story_specs: data.story_specs,
        design_specs: data.design_specs,
        work_items: data.work_items,
        workspace_sessions: data.workspace_sessions,
      });
    }
    return jsonResponse({});
  });
}

function jsonResponse(body: unknown) {
  return Promise.resolve(jsonResponseValue(body));
}

function jsonResponseValue(body: unknown) {
  return new Response(JSON.stringify(body), { status: 200 });
}

function projectsBody() {
  return {
    projects: [projectRecord("project_0001", "Aria")],
  };
}

function projectRecord(projectId: string, name: string, description: string | null = null) {
  return {
    project_id: projectId,
    name,
    description,
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
    last_opened_at: null,
  };
}

function repositoryRecord(overrides?: Partial<ReturnType<typeof repositoryRecordShape>>) {
  return {
    ...repositoryRecordShape(),
    ...(overrides ?? {}),
  };
}

function repositoryRecordShape() {
  return {
    repository_id: "repository_0001",
    project_id: "project_0001",
    name: "Aria Repo",
    path: "/tmp/aria",
    repo_hash: "hash",
    runtime_root: "/tmp/aria/.aria/runtime",
    default_policy_preset: "manual-write",
    default_provider_mode: "fake",
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  };
}

function workspaceSessionRecord(
  workspaceType: "story" | "design" | "work_item",
  entityId: string,
  sessionId: string,
  overrides?: Partial<WorkspaceSession>,
): WorkspaceSession {
  return {
    ...workspaceSessionRecordShape(workspaceType, entityId, sessionId),
    ...(overrides ?? {}),
  };
}

function workspaceSessionRecordShape(
  workspaceType: "story" | "design" | "work_item",
  entityId: string,
  sessionId: string,
): WorkspaceSession {
  return {
    workspace_session_id: sessionId,
    issue_id: "issue_0001",
    entity_id: entityId,
    workspace_type: workspaceType,
    status: "waiting_for_human",
    author_provider: "codex",
    reviewer_provider: "claude_code",
    review_rounds: 2,
    superpowers_enabled: true,
    openspec_enabled: true,
    messages: [],
  };
}

function initialLifecycleData(
  issueId: string,
  duplicate: boolean | undefined,
  empty: boolean | undefined,
): MockLifecycleData {
  if (empty) {
    return {
      story_specs: [],
      design_specs: [],
      work_items: [],
      workspace_sessions: [],
    };
  }

  const storyId = duplicate ? "shared_id" : "story_spec_0001";
  return {
    story_specs: [
      {
        story_spec_id: storyId,
        issue_id: issueId,
        repository_id: "repository_0001",
        title: duplicate ? "重复 ID Story" : "会话过期提示",
        current_version: 1,
        current_markdown_preview: "## 功能需求\n\n[REQ-001] 显示会话过期提示。",
        confirmation_status: "confirmed",
        artifact_versions: [],
      },
    ],
    design_specs: [
      {
        design_spec_id: "design_spec_0001",
        issue_id: issueId,
        story_spec_ids: [storyId],
        design_kind: "frontend",
        title: "前端提示设计",
        current_version: 1,
        current_markdown_preview: "## 关键决策\n\n[DEC-001] 使用全局提示条。",
        confirmation_status: "confirmed",
        artifact_versions: [],
      },
    ],
    work_items: [
      {
        work_item_id: "work_item_0001",
        issue_id: issueId,
        repository_id: "repository_0001",
        story_spec_ids: ["story_spec_0001"],
        design_spec_ids: ["design_spec_0001"],
        title: "实现提示组件",
        plan_status: "draft",
        execution_status: "planning",
      },
    ],
    workspace_sessions: [
      workspaceSessionRecord("story", storyId, "workspace_session_story_0001"),
      workspaceSessionRecord("design", "design_spec_0001", "workspace_session_design_0001"),
      workspaceSessionRecord("work_item", "work_item_0001", "workspace_session_work_item_0001"),
    ],
  };
}

function findSession(
  lifecycles: Map<
    string,
    {
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) => candidate.workspace_session_id === sessionId,
    );
    if (session) {
      return session;
    }
  }
  return null;
}

function findStoryBySession(
  lifecycles: Map<
    string,
    {
      story_specs: Array<Record<string, unknown>>;
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) => candidate.workspace_session_id === sessionId && candidate.workspace_type === "story",
    );
    if (session) {
      return lifecycle.story_specs.find((story) => story.story_spec_id === session.entity_id) ?? null;
    }
  }
  return null;
}

function findDesignBySession(
  lifecycles: Map<
    string,
    {
      design_specs: Array<Record<string, unknown>>;
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) => candidate.workspace_session_id === sessionId && candidate.workspace_type === "design",
    );
    if (session) {
      return lifecycle.design_specs.find((design) => design.design_spec_id === session.entity_id) ?? null;
    }
  }
  return null;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}
