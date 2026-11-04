import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";
import { deferred } from "./IssueLifecycleWorkbench.test-utils";

function logicalCodebase(
  overrides: Partial<{
    id: string;
    name: string;
    member_count: number;
  }> = {},
) {
  return {
    id: overrides.id ?? "lc_0001",
    name: overrides.name ?? "monorepo",
    kind: "logical" as const,
    repository_id: null,
    logical_codebase_id: overrides.id ?? "lc_0001",
    member_count: overrides.member_count ?? 2,
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

describe("CreateLifecycleIssueDialog", () => {
  it("shows submit errors and prevents duplicate submissions while pending", async () => {
    const submit = deferred<void>();
    const onCreate = vi.fn(() => submit.promise);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[logicalCodebase()]}
        listMembers={vi.fn().mockResolvedValue([member("lr-1", "api", "repository_1001")])}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "新增安全提示");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
    const group = await screen.findByRole("group", { name: "成员" });
    await user.click(within(group).getByRole("checkbox", { name: /api/ }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledTimes(1);

    submit.reject(new Error("create issue failed"));
    expect(await screen.findByText("create issue failed")).toBeInTheDocument();
  });
});

describe("CreateLifecycleIssueDialog 代码库选择（REQ-MRE-01）", () => {
  it("无「单仓库代码库」选项：下拉只列逻辑代码库", async () => {
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[
          logicalCodebase(),
          {
            id: "repo_0001",
            name: "Aria Repo",
            kind: "single_repo" as const,
            repository_id: "repository_0001",
            logical_codebase_id: null,
            member_count: null,
          },
        ]}
        listMembers={vi.fn().mockResolvedValue([])}
        onCreate={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />,
    );

    const select = screen.getByLabelText("代码库");
    const options = within(select).getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual([
      "请选择",
      "monorepo · 逻辑",
    ]);
    expect(screen.queryByText(/单仓/)).toBeNull();
  });

  it("未选逻辑代码库提交被拒：不调用 onCreate（不落库）", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[logicalCodebase()]}
        listMembers={vi.fn().mockResolvedValue([])}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "缺归属需求");
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByText("请选择逻辑代码库")).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("选择逻辑代码库时加载 active 成员并提交 logical_codebase_id + primary", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      member("lr-1", "api", "repository_1001"),
      member("lr-2", "legacy", null),
      member("lr-3", "web", "repository_1003", "removed"),
    ]);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
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

  it("逻辑代码库未勾选成员时阻止提交", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      member("lr-1", "api", "repository_1001"),
    ]);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
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
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");

    expect(
      await screen.findByText(
        "成员加载失败：logical codebase members unavailable",
      ),
    ).toBeInTheDocument();
    expect(listMembers).toHaveBeenCalledWith("lc_0001");
  });
});

describe("CreateLifecycleIssueDialog 基准分支（REQ-PIB-01）", () => {
  it("多仓路径不渲染基准分支选择器，payload 恒带 base_branch=null", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      member("lr-1", "api", "repository_1001"),
    ]);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[logicalCodebase()]}
        listMembers={listMembers}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "多仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
    const group = await screen.findByRole("group", { name: "成员" });
    await user.click(within(group).getByRole("checkbox", { name: /api/ }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(screen.queryByLabelText(/^基准分支/, { selector: "select" })).toBeNull();
    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({ base_branch: null }),
    );
  });
});

describe("CreateLifecycleIssueDialog 成员复选（REQ-MRE-01）", () => {
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
        codebases={[logicalCodebase({ member_count: 3 })]}
        listMembers={listMembers}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
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

  it("勾选恰 1 个成员与勾选多个走完全相同的提交路径（单成员 ≡ 单选）", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const listMembers = vi.fn().mockResolvedValue([
      member("lr-api", "api", "repository_1001"),
      member("lr-web", "web", "repository_1002"),
    ]);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        codebases={[logicalCodebase({ member_count: 2 })]}
        listMembers={listMembers}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "单成员需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
    const group = await screen.findByRole("group", { name: "成员" });
    await user.click(within(group).getByRole("checkbox", { name: /web · repository_1002/ }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    // 单成员与多成员同一条路径：同样携带 logical_codebase_id + focus_repository_ids。
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
        codebases={[logicalCodebase()]}
        listMembers={vi.fn().mockResolvedValue([member("lr-api", "api", "repository_1001")])}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(screen.getByLabelText("代码库"), "lc_0001");
    await screen.findByRole("group", { name: "成员" });
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByText("请选择成员")).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });
});
