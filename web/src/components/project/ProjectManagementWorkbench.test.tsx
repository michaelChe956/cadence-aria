import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProjectManagementWorkbench } from "./ProjectManagementWorkbench";

describe("ProjectManagementWorkbench", () => {
  it("renders active project issues across lifecycle states with execution workspace artifacts", async () => {
    vi.stubGlobal("fetch", productWorkbenchFetch());
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    expect(await screen.findByRole("main", { name: "任务管理页面" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "切换到 Aria Project" })).toHaveAttribute(
      "aria-current",
      "true",
    );
    expect(screen.getByRole("navigation", { name: "Project 选择" })).toHaveTextContent(
      "Aria Project",
    );
    expect(screen.getByRole("navigation", { name: "Project 选择" })).toHaveTextContent(
      "Other Project",
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
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "Story Spec",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "Design Spec",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "Aria Core",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "Story Spec 产物",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "Design Spec 产物",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "Work Item 产物",
    );

    await user.click(screen.getByRole("button", { name: "实现任务卡片" }));
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "spec_story_issue_0003",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "design_issue_0003",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "coding_report_issue_0003",
    );
    expect(screen.getByRole("region", { name: "Issue 执行 Workspace" })).toHaveTextContent(
      "final_summary_issue_0003",
    );
  });

  it("creates issues in the active project and starts them with one execution workspace", async () => {
    const fetchSpy = productWorkbenchFetch();
    vi.stubGlobal("fetch", fetchSpy);
    const onOpenExecution = vi.fn();
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={onOpenExecution} />);

    await screen.findByRole("navigation", { name: "Project 选择" });
    await user.click(screen.getByRole("button", { name: "新建 Issue" }));
    const createDialog = screen.getByRole("dialog", { name: "新建 Issue" });
    expect(within(createDialog).getByLabelText("代码库")).toHaveDisplayValue(
      "Aria Core · repository_0001",
    );
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
            repository_id: "repository_0001",
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
    expect(within(runDialog).getByLabelText("运行 Workspace")).toHaveDisplayValue(
      "Aria Core · repository_0001",
    );
    expect(within(runDialog).queryByText("Other Project Workspace")).not.toBeInTheDocument();
    await user.click(within(runDialog).getByRole("button", { name: "开始运行" }));

    await waitFor(() =>
      expect(fetchSpy).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues/issue_0005/start",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ workspace_id: "repository_0001" }),
        }),
      ),
    );
    expect(onOpenExecution).toHaveBeenCalledWith({
      issueId: "issue_0005",
      workspaceId: "product:project_0001:repository_0001",
      taskId: "task_0001",
    });
  });

  it("opens separate dialogs for project management and code repository creation", async () => {
    const fetchSpy = productWorkbenchFetch();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    await screen.findByRole("navigation", { name: "Project 选择" });
    await user.click(screen.getByRole("button", { name: "管理 Project" }));
    expect(screen.getByRole("dialog", { name: "Project 管理" })).toBeInTheDocument();

    await user.click(
      within(screen.getByRole("region", { name: "Issue 执行 Workspace" })).getByRole("button", {
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

  it("exposes delete actions for projects issues and code repositories", async () => {
    const fetchSpy = productWorkbenchFetch();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    await screen.findByRole("navigation", { name: "Project 选择" });
    await user.click(screen.getByRole("button", { name: "删除 Issue 澄清登录流程" }));
    await user.click(screen.getByRole("button", { name: "删除代码库 Aria Core" }));
    await user.click(screen.getByRole("button", { name: "删除 Project Aria Project" }));

    await waitFor(() =>
      expect(fetchSpy).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues/issue_0001",
        expect.objectContaining({ method: "DELETE" }),
      ),
    );
    expect(fetchSpy).toHaveBeenCalledWith(
      "/api/projects/project_0001/repositories/repository_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(fetchSpy).toHaveBeenCalledWith(
      "/api/projects/project_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("opens an already started issue directly in its existing workspace", async () => {
    const fetchSpy = productWorkbenchFetch();
    vi.stubGlobal("fetch", fetchSpy);
    const onOpenExecution = vi.fn();
    const user = userEvent.setup();

    render(<ProjectManagementWorkbench onOpenExecution={onOpenExecution} />);

    await screen.findByRole("button", { name: "切换到 Aria Project" });
    await user.click(screen.getByRole("button", { name: "实现任务卡片" }));
    await user.click(screen.getAllByRole("button", { name: "运行 Issue" })[0]);

    expect(screen.queryByRole("dialog", { name: "运行 Issue" })).not.toBeInTheDocument();
    expect(onOpenExecution).toHaveBeenCalledWith({
      issueId: "issue_0003",
      workspaceId: "product:project_0001:repository_0001",
      taskId: "task_0003",
    });
  });

});

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}

function productWorkbenchFetch() {
  const projects = [
    {
      project_id: "project_0001",
      name: "Aria Project",
      description: "当前项目",
      created_at: "2026-05-15T00:00:00Z",
      updated_at: "2026-05-15T00:00:00Z",
      last_opened_at: "2026-05-15T00:00:00Z",
    },
    {
      project_id: "project_0002",
      name: "Other Project",
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
        name: "Other Project Workspace",
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
          name: "New Project",
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
    workspace_id: repoId ? `product:project_0001:${repoId}` : null,
    task_id: repoId ? `task_${issueId.replace("issue_", "")}` : null,
    session_id: repoId ? `session_${issueId.replace("issue_", "")}` : null,
    title,
    description: `${title} 的详细说明`,
    change_id: title.toLowerCase().replaceAll(" ", "-"),
    phase,
    status,
    active_binding_id: status === "draft" ? null : "binding_0001",
    artifacts: repoId
      ? [
          issueArtifact(issueId, "story_spec", `spec_story_${issueId}`, "spec", "N05"),
          issueArtifact(issueId, "design_spec", `design_${issueId}`, "design", "N08"),
          issueArtifact(issueId, "work_item", `coding_report_${issueId}`, "coding_report", "N16"),
          issueArtifact(issueId, "done", `final_summary_${issueId}`, "final_summary", "N27"),
        ]
      : [],
    created_at: "2026-05-15T00:00:00Z",
    updated_at: "2026-05-15T00:00:00Z",
  };
}

function issueArtifact(
  issueId: string,
  stage: string,
  artifactRef: string,
  artifactKind: string,
  producerNode: string,
) {
  return {
    artifact_ref: artifactRef,
    artifact_kind: artifactKind,
    producer_node: producerNode,
    path: `.aria/runtime/tasks/task_${issueId.replace("issue_", "")}/artifacts/${artifactRef}.json`,
    summary: artifactKind,
    stage,
  };
}
