import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProjectManagementWorkbench } from "./ProjectManagementWorkbench";

describe("ProjectManagementWorkbench", () => {
  it("renders active workspace issues across the four lifecycle states with spec artifacts", async () => {
    vi.stubGlobal("fetch", productWorkbenchFetch());

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    expect(await screen.findByRole("main", { name: "任务管理页面" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Workspace 选择" })).toHaveTextContent(
      "Aria Workspace",
    );
    expect(screen.getByRole("navigation", { name: "Workspace 选择" })).toHaveTextContent(
      "Other Workspace",
    );
    expect(screen.getByRole("button", { name: "切换到 Aria Workspace" })).toHaveAttribute(
      "aria-current",
      "true",
    );
    expect(screen.getByRole("region", { name: "Issue 生命周期看板" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Story Spec 阶段" })).toHaveTextContent(
      "澄清登录流程",
    );
    expect(screen.getByRole("region", { name: "Design Spec 阶段" })).toHaveTextContent(
      "设计权限模型",
    );
    expect(screen.getByRole("region", { name: "Work Item 阶段" })).toHaveTextContent(
      "实现任务卡片",
    );
    expect(screen.getByRole("region", { name: "Done 阶段" })).toHaveTextContent("完成执行闭环");
    expect(screen.getByRole("region", { name: "Issue 驱动 Workspace" })).toHaveTextContent(
      "Story Spec",
    );
    expect(screen.getByRole("region", { name: "Issue 驱动 Workspace" })).toHaveTextContent(
      "Design Spec",
    );
    expect(screen.getByRole("region", { name: "Issue 驱动 Workspace" })).toHaveTextContent(
      "Aria Core",
    );
  });

  it("creates issues in the active workspace and starts them with one repository from that workspace", async () => {
    const fetchSpy = productWorkbenchFetch();
    vi.stubGlobal("fetch", fetchSpy);
    const onOpenExecution = vi.fn();
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={onOpenExecution} />);

    await screen.findByRole("navigation", { name: "Workspace 选择" });
    await user.click(screen.getByRole("button", { name: "新建 Issue" }));
    const createDialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(within(createDialog).getByLabelText("Issue 标题"), "新增计费设置");
    await user.type(within(createDialog).getByLabelText("Issue 描述"), "需要先确认 story spec");
    await user.click(within(createDialog).getByRole("button", { name: "创建" }));

    await waitFor(() =>
      expect(fetchSpy).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({
            title: "新增计费设置",
            description: "需要先确认 story spec",
            change_id: null,
          }),
        }),
      ),
    );
    expect(
      await within(screen.getByRole("region", { name: "Story Spec 阶段" })).findByText(
        "新增计费设置",
      ),
    ).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "运行 Issue" })[0]);
    const runDialog = screen.getByRole("dialog", { name: "运行 Issue" });
    expect(within(runDialog).getByLabelText("运行代码库")).toHaveDisplayValue(
      "Aria Core · repository_0001",
    );
    expect(within(runDialog).queryByText("Other Workspace Repo")).not.toBeInTheDocument();
    await user.click(within(runDialog).getByRole("button", { name: "开始运行" }));

    await waitFor(() =>
      expect(fetchSpy).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues/issue_0005/start",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ repository_id: "repository_0001" }),
        }),
      ),
    );
    expect(onOpenExecution).toHaveBeenCalledWith({
      issueId: "issue_0005",
      workspaceId: "product:project_0001:repository_0001",
      taskId: "task_0001",
    });
  });

  it("opens separate dialogs for workspace management and repository creation", async () => {
    const fetchSpy = productWorkbenchFetch();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    await screen.findByRole("navigation", { name: "Workspace 选择" });
    await user.click(screen.getByRole("button", { name: "管理 Workspace" }));
    expect(screen.getByRole("dialog", { name: "Workspace 管理" })).toBeInTheDocument();

    await user.click(
      within(screen.getByRole("region", { name: "Issue 驱动 Workspace" })).getByRole("button", {
        name: "添加代码库",
      }),
    );
    const repoDialog = screen.getByRole("dialog", { name: "添加代码库" });
    await user.type(within(repoDialog).getByLabelText("代码库名称"), "Aria Docs");
    await user.type(within(repoDialog).getByLabelText("代码库路径"), "/tmp/aria-docs");
    await user.click(within(repoDialog).getByRole("button", { name: "添加" }));

    await waitFor(() =>
      expect(fetchSpy).toHaveBeenCalledWith(
        "/api/projects/project_0001/repositories",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({
            name: "Aria Docs",
            path: "/tmp/aria-docs",
            default_policy_preset: null,
            default_provider_mode: null,
          }),
        }),
      ),
    );
  });

});

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}

function productWorkbenchFetch() {
  const projects = [
    {
      project_id: "project_0001",
      name: "Aria Workspace",
      description: "当前激活空间",
      created_at: "2026-05-15T00:00:00Z",
      updated_at: "2026-05-15T00:00:00Z",
      last_opened_at: "2026-05-15T00:00:00Z",
    },
    {
      project_id: "project_0002",
      name: "Other Workspace",
      description: null,
      created_at: "2026-05-15T00:00:00Z",
      updated_at: "2026-05-15T00:00:00Z",
      last_opened_at: null,
    },
  ];
  const repositoriesByProject = {
    project_0001: [
      {
        repository_id: "repository_0001",
        project_id: "project_0001",
        name: "Aria Core",
        path: "/tmp/aria-core",
        repo_hash: "hash_core",
        runtime_root: "/tmp/aria-core/.aria/runtime",
        default_policy_preset: "manual-write",
        default_provider_mode: "fake",
        created_at: "2026-05-15T00:00:00Z",
        updated_at: "2026-05-15T00:00:00Z",
      },
    ],
    project_0002: [
      {
        repository_id: "repository_0001",
        project_id: "project_0002",
        name: "Other Workspace Repo",
        path: "/tmp/other-repo",
        repo_hash: "hash_other",
        runtime_root: "/tmp/other-repo/.aria/runtime",
        default_policy_preset: "manual-write",
        default_provider_mode: "fake",
        created_at: "2026-05-15T00:00:00Z",
        updated_at: "2026-05-15T00:00:00Z",
      },
    ],
  };
  const issues = [
    productIssue("issue_0001", "澄清登录流程", "clarification", "draft", null),
    productIssue("issue_0002", "设计权限模型", "clarification", "in_progress", null),
    productIssue("issue_0003", "实现任务卡片", "development", "in_progress", "repository_0001"),
    productIssue("issue_0004", "完成执行闭环", "development", "completed", "repository_0001"),
  ];

  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/projects") {
      if (init?.method === "POST") {
        return jsonResponse({
          project_id: "project_0003",
          name: "New Workspace",
          description: null,
          created_at: "2026-05-15T00:00:00Z",
          updated_at: "2026-05-15T00:00:00Z",
          last_opened_at: null,
        });
      }
      return jsonResponse({ projects });
    }
    if (url === "/api/projects/project_0001/repositories") {
      if (init?.method === "POST") {
        return jsonResponse({
          repository_id: "repository_0002",
          project_id: "project_0001",
          name: "Aria Docs",
          path: "/tmp/aria-docs",
          repo_hash: "hash_docs",
          runtime_root: "/tmp/aria-docs/.aria/runtime",
          default_policy_preset: "manual-write",
          default_provider_mode: "fake",
          created_at: "2026-05-15T00:00:00Z",
          updated_at: "2026-05-15T00:00:00Z",
        });
      }
      return jsonResponse({ repositories: repositoriesByProject.project_0001 });
    }
    if (url === "/api/projects/project_0002/repositories") {
      return jsonResponse({ repositories: repositoriesByProject.project_0002 });
    }
    if (url === "/api/projects/project_0001/issues") {
      if (init?.method === "POST") {
        return jsonResponse(
          productIssue("issue_0005", "新增计费设置", "clarification", "draft", null),
        );
      }
      return jsonResponse({ issues });
    }
    if (url === "/api/projects/project_0001/issues/issue_0001/start") {
      return jsonResponse({
        issue_id: "issue_0001",
        project_id: "project_0001",
        repository_id: "repository_0001",
        workspace_id: "product:project_0001:repository_0001",
        task_id: "task_0001",
        session_id: "session_0001",
        status: "in_progress",
      });
    }
    if (url === "/api/projects/project_0001/issues/issue_0005/start") {
      return jsonResponse({
        issue_id: "issue_0005",
        project_id: "project_0001",
        repository_id: "repository_0001",
        workspace_id: "product:project_0001:repository_0001",
        task_id: "task_0001",
        session_id: "session_0001",
        status: "in_progress",
      });
    }
    return jsonResponse({});
  });
}

function productIssue(
  issueId: string,
  title: string,
  phase: string,
  status: string,
  repoId: string | null,
) {
  return {
    issue_id: issueId,
    project_id: "project_0001",
    repo_id: repoId,
    title,
    description: `${title} 的详细说明`,
    change_id: title.toLowerCase().replaceAll(" ", "-"),
    phase,
    status,
    active_binding_id: status === "draft" ? null : "binding_0001",
    created_at: "2026-05-15T00:00:00Z",
    updated_at: "2026-05-15T00:00:00Z",
  };
}
