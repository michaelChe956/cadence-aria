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
        repositories={[repositoryRecord()]}
        codebases={[]}
        listMembers={vi.fn().mockResolvedValue([])}
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
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
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
    });
  });

  it("单仓库代码库提交 logical_codebase_id 为 null", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        repositories={[repositoryRecord()]}
        codebases={[]}
        listMembers={vi.fn()}
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
        repositories={[repositoryRecord()]}
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
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
});
