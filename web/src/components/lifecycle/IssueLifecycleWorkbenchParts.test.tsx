import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  workItemRecord,
  workItemRepositoryGroupRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

describe("IssueLifecycleWorkbenchParts work item repository grouping (REQ-TGT-05)", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("renders work items grouped by target repository with alias and status", async () => {
    const groups = [
      workItemRepositoryGroupRecord({
        target_repository_id: "repo_api",
        alias: "api",
        status: "pending",
        compatibility_projection: false,
        items: [
          workItemRecord({
            work_item_id: "work_item_backend",
            issue_id: "issue_0001",
            title: "后端 API 实现",
            kind: "backend",
            plan_status: "confirmed",
            execution_status: "pending",
          }),
        ],
      }),
      workItemRepositoryGroupRecord({
        target_repository_id: "repo_web",
        alias: "web",
        status: "completed",
        compatibility_projection: false,
        items: [
          workItemRecord({
            work_item_id: "work_item_frontend",
            issue_id: "issue_0001",
            title: "前端 UI 实现",
            kind: "frontend",
            plan_status: "confirmed",
            execution_status: "completed",
          }),
        ],
      }),
    ];
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ workItemRepositoryGroups: groups }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    // 每组标注仓库名（alias）。
    expect(workItemRegion).toHaveTextContent("api");
    expect(workItemRegion).toHaveTextContent("web");
    // 每组标注仓库级聚合状态。
    expect(workItemRegion).toHaveTextContent("pending");
    expect(workItemRegion).toHaveTextContent("completed");
    // 组内 Work Item 标题可见。
    expect(workItemRegion).toHaveTextContent("后端 API 实现");
    expect(workItemRegion).toHaveTextContent("前端 UI 实现");
  });

  it("marks the legacy unassigned group as compatibility projection", async () => {
    const groups = [
      workItemRepositoryGroupRecord({
        target_repository_id: null,
        alias: "未指定仓库",
        status: "blocked",
        compatibility_projection: true,
        items: [
          workItemRecord({
            work_item_id: "work_item_legacy",
            issue_id: "issue_0001",
            title: "遗留 Work Item",
            kind: "backend",
            plan_status: "confirmed",
            execution_status: "blocked",
          }),
        ],
      }),
    ];
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ workItemRepositoryGroups: groups }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    expect(workItemRegion).toHaveTextContent("未指定仓库");
    expect(workItemRegion).toHaveTextContent("blocked");
    expect(workItemRegion).toHaveTextContent("兼容投影");
    expect(workItemRegion).toHaveTextContent("遗留 Work Item");
  });

  it("keeps flat work item cards when work_item_repository_groups is empty (single repo compatibility)", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ workItemRepositoryGroups: [] }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    // 空分组（单仓/无分组）回退扁平展示：仍展示 Work Item Group 卡片。
    expect(workItemRegion).toHaveTextContent("Work Item Group");
  });
});
