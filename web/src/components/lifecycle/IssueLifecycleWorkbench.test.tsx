import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";

describe("IssueLifecycleWorkbench", () => {
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

  it("opens provider workspace sessions from derived lifecycle cards", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "会话过期提示" }));

    expect(await screen.findByRole("dialog", { name: "Story Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Workspace 产物" })).toHaveTextContent(
      "workspace_session_story_0001",
    );

    await user.type(screen.getByLabelText("补充指令"), "请补充验收标准");
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/workspace-sessions/workspace_session_story_0001/message",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ role: "user", content: "请补充验收标准" }),
      }),
    );

    await user.click(screen.getByRole("button", { name: "关闭" }));
    await user.click(screen.getByRole("button", { name: "实现提示组件" }));

    expect(await screen.findByRole("dialog", { name: "Work Item Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Workspace 流程" })).toHaveTextContent(
      "author plan",
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
  invalidLifecycle?: boolean;
  issueTitles?: string[];
  issueTitlesByProject?: Record<string, string>;
  projects?: Array<ReturnType<typeof projectRecord>>;
  projectResponses?: Array<Promise<Response>>;
}) {
  const projects = [...(options?.projects ?? [projectRecord("project_0001", "Aria")])];
  let projectCall = 0;
  const issueCallsByProject = new Map<string, number>();
  const latestIssueTitlesByProject = new Map<string, string>();
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
    if (url === "/api/projects") {
      const response = options?.projectResponses?.[projectCall];
      projectCall += 1;
      return response ?? jsonResponse({ projects });
    }
    const repositoryMatch = url.match(/^\/api\/projects\/([^/]+)\/repositories$/);
    if (repositoryMatch) {
      return jsonResponse({
        repositories: repositoryMatch[1] === "project_0001" ? [repositoryRecord()] : [],
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
      if (projectId !== "project_0001") {
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
            issue_id: options?.duplicateCardIds ? "shared_id" : "issue_0001",
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
      const storyId = duplicate ? "shared_id" : "story_spec_0001";
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
        story_specs: [
          {
            story_spec_id: storyId,
            issue_id: issueId,
            repository_id: "repository_0001",
            title: duplicate ? "重复 ID Story" : "会话过期提示",
            current_version: 1,
            confirmation_status: "confirmed",
          },
        ],
        design_specs: [
          {
            design_spec_id: "design_spec_0001",
            issue_id: issueId,
            story_spec_ids: [duplicate ? "shared_id" : "story_spec_0001"],
            design_kind: "frontend",
            title: "前端提示设计",
            current_version: 1,
            confirmation_status: "confirmed",
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
          workspaceSessionRecord(
            "work_item",
            "work_item_0001",
            "workspace_session_work_item_0001",
          ),
        ],
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

function repositoryRecord() {
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
) {
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

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}
