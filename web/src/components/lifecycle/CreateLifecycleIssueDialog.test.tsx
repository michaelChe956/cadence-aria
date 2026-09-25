import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";
import { deferred, repositoryRecord } from "./IssueLifecycleWorkbench.test-utils";

describe("CreateLifecycleIssueDialog", () => {
  it("shows submit errors and prevents duplicate submissions while pending", async () => {
    const submit = deferred<void>();
    const onCreate = vi.fn(() => submit.promise);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[]}
        listMembers={vi.fn().mockResolvedValue([])}
        listBranches={vi
          .fn()
          .mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "新增安全提示");
    await user.selectOptions(
      screen.getByLabelText("代码库"),
      "repo:repository_0001",
    );
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledTimes(1);

    submit.reject(new Error("create issue failed"));
    expect(await screen.findByText("create issue failed")).toBeInTheDocument();
  });
});

describe("CreateLifecycleIssueDialog 代码库选择（R8）", () => {
  function logicalCodebase() {
    return {
      id: "lc_0001",
      name: "monorepo",
      kind: "logical" as const,
      repository_id: null,
      logical_codebase_id: "lc_0001",
      member_count: 2,
    };
  }

  it("选择逻辑代码库时加载 active 成员并提交 logical_codebase_id + primary", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      {
        logical_repository_id: "lr-1",
        physical_repository_id: "repository_1001",
        alias: "api",
        status: "active" as const,
      },
      {
        logical_repository_id: "lr-2",
        physical_repository_id: null,
        alias: "legacy",
        status: "active" as const,
      },
      {
        logical_repository_id: "lr-3",
        physical_repository_id: "repository_1003",
        alias: "web",
        status: "removed" as const,
      },
    ]);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        listBranches={vi.fn().mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc:lc_0001");
    expect(listMembers).toHaveBeenCalledWith("lc_0001");

    const primarySelect = await screen.findByLabelText("Primary 成员");
    const option = screen.getByRole(
      "option",
      { name: /legacy/ },
    ) as HTMLOptionElement;
    expect(option.disabled).toBe(true);
    expect(screen.queryByRole("option", { name: /web/ })).toBeNull();

    await user.selectOptions(primarySelect, "repository_1001");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "跨仓需求",
      description: null,
      repository_id: "repository_1001",
      logical_codebase_id: "lc_0001",
      base_branch: null,
    });
  });

  it("单仓库代码库提交 logical_codebase_id 为 null", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[]}
        listMembers={vi.fn()}
        listBranches={vi.fn().mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "单仓需求");
    await user.selectOptions(
      screen.getByLabelText("代码库"),
      "repo:repository_0001",
    );
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "单仓需求",
      description: null,
      repository_id: "repository_0001",
      logical_codebase_id: null,
      base_branch: "main",
    });
  });

  it("逻辑代码库未选 primary 时阻止提交", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      {
        logical_repository_id: "lr-1",
        physical_repository_id: "repository_1001",
        alias: "api",
        status: "active" as const,
      },
    ]);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        listBranches={vi.fn().mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc:lc_0001");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByText("请选择 Primary 成员")).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("M3：listMembers 失败时展示错误提示而非静默清空成员", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi
      .fn()
      .mockRejectedValue(new Error("logical codebase members unavailable"));
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        listBranches={vi.fn().mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.selectOptions(screen.getByLabelText("代码库"), "lc:lc_0001");

    expect(
      await screen.findByText(
        "成员加载失败：logical codebase members unavailable",
      ),
    ).toBeInTheDocument();
    expect(listMembers).toHaveBeenCalledWith("lc_0001");
  });
});

describe("CreateLifecycleIssueDialog 基准分支选择（REQ-PIB-01）", () => {
  it("单仓默认分支预选：选择器加载列表并预选服务端 default_branch，payload 携带所选分支", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listBranches = vi.fn().mockResolvedValue({
      branches: ["feature/x", "main", "release/1.0"],
      default_branch: "main",
    });
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[]}
        listMembers={vi.fn()}
        listBranches={listBranches}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    expect(listBranches).not.toHaveBeenCalled();
    await user.type(screen.getByLabelText("Issue 标题"), "分支锚定");
    await user.selectOptions(
      screen.getByLabelText("代码库"),
      "repo:repository_0001",
    );
    expect(listBranches).toHaveBeenCalledWith("project_0001", "repository_0001");

    const branchSelect = await screen.findByLabelText(/^基准分支/, { selector: "select" });
    expect(branchSelect).toHaveValue("main");

    await user.selectOptions(branchSelect, "feature/x");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "分支锚定",
      description: null,
      repository_id: "repository_0001",
      logical_codebase_id: null,
      base_branch: "feature/x",
    });
  });

  it("仓库无 main/master（default_branch=null）：无默认值且未显式选择即阻止提交", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[]}
        listMembers={vi.fn()}
        listBranches={vi
          .fn()
          .mockResolvedValue({ branches: ["release/1.0"], default_branch: null })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "皆无默认");
    await user.selectOptions(
      screen.getByLabelText("代码库"),
      "repo:repository_0001",
    );
    const branchSelect = await screen.findByLabelText(/^基准分支/, { selector: "select" });
    expect(branchSelect).toHaveValue("");

    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    expect(
      await screen.findByText("仓库无 main/master 默认分支，请显式选择基准分支"),
    ).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();

    await user.selectOptions(branchSelect, "release/1.0");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    expect(onCreate).toHaveBeenCalledWith({
      title: "皆无默认",
      description: null,
      repository_id: "repository_0001",
      logical_codebase_id: null,
      base_branch: "release/1.0",
    });
  });

  it("逻辑代码库选择不渲染基准分支选择器（多仓差异基线为 Non-Goal）", async () => {
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[
          {
            id: "lc_0001",
            name: "monorepo",
            kind: "logical",
            repository_id: null,
            logical_codebase_id: "lc_0001",
            member_count: 1,
          },
        ]}
        listMembers={vi.fn().mockResolvedValue([
          {
            logical_repository_id: "lr-1",
            physical_repository_id: "repository_1001",
            alias: "api",
            status: "active" as const,
          },
        ])}
        listBranches={vi.fn()}
        onCreate={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />,
    );

    await user.selectOptions(screen.getByLabelText("代码库"), "lc:lc_0001");
    await screen.findByLabelText("Primary 成员");
    expect(screen.queryByLabelText(/^基准分支/, { selector: "select" })).toBeNull();
  });
});
