import { render, screen, within } from "@testing-library/react";
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

    const group = await screen.findByRole("group", { name: "成员" });
    // 无物理映射成员禁用；removed 成员不渲染。
    expect(within(group).getByRole("checkbox", { name: /legacy/ })).toBeDisabled();
    expect(within(group).queryByRole("checkbox", { name: /web/ })).toBeNull();

    await user.click(within(group).getByRole("checkbox", { name: /api · repository_1001/ }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "跨仓需求",
      description: null,
      repository_id: "repository_1001",
      logical_codebase_id: "lc_0001",
      base_branch: null,
      focus_repository_ids: ["lr-1"],
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
      focus_repository_ids: null,
    });
  });

  it("逻辑代码库未勾选成员时阻止提交", async () => {
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
    await screen.findByRole("group", { name: "成员" });
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByText("请选择成员")).toBeInTheDocument();
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
      focus_repository_ids: null,
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
      focus_repository_ids: null,
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
    await screen.findByRole("group", { name: "成员" });
    expect(screen.queryByLabelText(/^基准分支/, { selector: "select" })).toBeNull();
  });
});

describe("CreateLifecycleIssueDialog 成员复选（REQ-MRE-01）", () => {
  function logicalCodebase() {
    return {
      id: "lc_0001",
      name: "monorepo",
      kind: "logical" as const,
      repository_id: null,
      logical_codebase_id: "lc_0001",
      member_count: 3,
    };
  }

  function member(
    logicalId: string,
    alias: string,
    physicalId: string | null,
    status: "active" | "removed" = "active",
  ) {
    return {
      logical_repository_id: logicalId,
      physical_repository_id: physicalId,
      alias,
      status,
    };
  }

  it("LC 成员复选：勾选多个提交 focus_repository_ids=勾选集（列表序），首项承载 primary repository_id", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      member("lr-api", "api", "repository_1001"),
      member("lr-web", "web", "repository_1002"),
      member("lr-removed", "removed", "repository_1003", "removed"),
      member("lr-legacy", "legacy", null),
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
    const group = await screen.findByRole("group", { name: "成员" });

    // 非 active 成员不渲染；无物理映射成员禁用（不可勾选）。
    expect(within(group).queryByRole("checkbox", { name: /removed/ })).toBeNull();
    expect(within(group).getByRole("checkbox", { name: /legacy/ })).toBeDisabled();

    // 故意乱序勾选：payload 仍按成员列表序（api, web）提交。
    await user.click(within(group).getByRole("checkbox", { name: /web · repository_1002/ }));
    await user.click(within(group).getByRole("checkbox", { name: /api · repository_1001/ }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "跨仓需求",
      description: null,
      repository_id: "repository_1001",
      logical_codebase_id: "lc_0001",
      base_branch: null,
      focus_repository_ids: ["lr-api", "lr-web"],
    });
  });

  it("勾选恰 1 个成员等价旧单选：repository_id=该成员物理仓，focus 仅含该成员", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      member("lr-api", "api", "repository_1001"),
      member("lr-web", "web", "repository_1002"),
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

    await user.type(screen.getByLabelText("Issue 标题"), "单成员需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc:lc_0001");
    const group = await screen.findByRole("group", { name: "成员" });
    await user.click(within(group).getByRole("checkbox", { name: /web · repository_1002/ }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "单成员需求",
      description: null,
      repository_id: "repository_1002",
      logical_codebase_id: "lc_0001",
      base_branch: null,
      focus_repository_ids: ["lr-web"],
    });
  });

  it("未勾选任何成员时阻止提交", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={vi.fn().mockResolvedValue([member("lr-api", "api", "repository_1001")])}
        listBranches={vi.fn().mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc:lc_0001");
    await screen.findByRole("group", { name: "成员" });
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByText("请选择成员")).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("单仓路径 payload 不携带成员勾选集（focus_repository_ids=null）", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        projectId="project_0001"
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={vi.fn()}
        listBranches={vi.fn().mockResolvedValue({ branches: ["main"], default_branch: "main" })}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "单仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "repo:repository_0001");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledWith({
      title: "单仓需求",
      description: null,
      repository_id: "repository_0001",
      logical_codebase_id: null,
      base_branch: "main",
      focus_repository_ids: null,
    });
  });
});
