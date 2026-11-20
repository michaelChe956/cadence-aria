import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProductIssue } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  projectRecord,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-utils";

// PR #92 复审 Important 回归：删除在途的行 busy 判定不得依赖生命周期详情已加载。
// handleDeleteIssue 发起删除时即生成 "issue:<issueId>:<issueId>" 删除键；若页面
// 只从已加载卡片反查 issueId，「生命周期待查看」（unloaded）组的行在 DELETE
// 在途期间不会进入 busy，删除按钮保持可用，连点可发出第二个 DELETE（第二次
// 会 issue not found 并留下可见错误）。要求：从删除键本身取回 issueId——未加载
// 行与已加载行在 DELETE 在途期间同样 aria-busy、按钮禁用、连点不重复出站。

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
};

function issueRecord(issueId: string, title: string): ProductIssue {
  return {
    issue_id: issueId,
    project_id: "project_0001",
    repo_id: "repository_0001",
    base_branch: null,
    workspace_id: null,
    task_id: null,
    session_id: null,
    title,
    description: "描述",
    change_id: `${issueId}-change`,
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  };
}

// issue_0001 的 lifecycle 读取永久挂起（行停留 unloaded 组）；issue_0002 正常
// 返回（行进入已加载分组）。DELETE 一律挂起、由测试按需结算并计数出站次数。
function deletePendingGateFetch() {
  const heldDetailIssueId = "issue_0001";
  const deletedIssueIds = new Set<string>();
  const issues = [
    issueRecord("issue_0001", "详情在途议题"),
    issueRecord("issue_0002", "详情已就绪议题"),
  ];
  const detailGate = deferred<Response>();
  const deleteGates = new Map<string, Deferred<Response>>();
  const deleteRequests = new Map<string, number>();

  async function baseFetch(
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> {
    void init;
    const url = String(input);
    if (url === "/api/projects") {
      return jsonResponse({ projects: [projectRecord("project_0001", "Aria")] });
    }
    if (url === "/api/projects/project_0001/repositories") {
      return jsonResponse({ repositories: [repositoryRecord()] });
    }
    if (url === "/api/projects/project_0001/codebases") {
      return jsonResponse({ codebases: [] });
    }
    if (url === "/api/projects/project_0001/issues") {
      return jsonResponse({
        issues: issues.filter((issue) => !deletedIssueIds.has(issue.issue_id)),
      });
    }
    const lifecycleMatch = url.match(
      /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^/]+)$/,
    );
    if (lifecycleMatch) {
      const issue = issues.find(
        (candidate) => candidate.issue_id === lifecycleMatch[1],
      );
      return jsonResponse({
        issue,
        story_specs: [],
        design_specs: [],
        work_item_plans: [],
        work_items: [],
        work_item_repository_groups: [],
        workspace_sessions: [],
        coding_attempts: [],
      });
    }
    return jsonResponse({});
  }

  const fetchMock = vi.fn(
    (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const url = String(input);
      const lifecycleMatch = url.match(
        /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^/]+)$/,
      );
      if (lifecycleMatch && lifecycleMatch[1] === heldDetailIssueId) {
        return detailGate.promise;
      }
      const issueDeleteMatch = url.match(
        /^\/api\/projects\/([^/]+)\/issues\/([^/?]+)$/,
      );
      if (issueDeleteMatch && init?.method === "DELETE") {
        const issueId = issueDeleteMatch[2];
        deleteRequests.set(issueId, (deleteRequests.get(issueId) ?? 0) + 1);
        let gate = deleteGates.get(issueId);
        if (!gate) {
          gate = deferred<Response>();
          deleteGates.set(issueId, gate);
        }
        return gate.promise;
      }
      return baseFetch(input, init);
    },
  );

  return {
    fetchMock,
    deleteRequestCount: (issueId: string) => deleteRequests.get(issueId) ?? 0,
    // 结算 DELETE：按服务端删除回执语义，后续 issues 列表读取剔除该行。
    releaseDelete: (issueId: string) => {
      const gate = deleteGates.get(issueId);
      if (!gate) {
        throw new Error(`no in-flight DELETE for ${issueId}`);
      }
      deletedIssueIds.add(issueId);
      gate.resolve(jsonResponse({ status: "deleted" }));
    },
  };
}

function queueRegion() {
  return screen.getByRole("region", { name: "Issue 卡片列表" });
}

function queueRow(issueId: string) {
  const row = queueRegion().querySelector(
    `[data-testid="issue-queue-row"][data-issue-id="${issueId}"]`,
  );
  if (!row) {
    throw new Error(`queue row not rendered: ${issueId}`);
  }
  return row as HTMLElement;
}

function rowGroupKey(issueId: string) {
  return (
    queueRow(issueId)
      .closest('[data-testid="issue-queue-group-rows"]')
      ?.getAttribute("data-group-key") ?? null
  );
}

describe("IssueLifecycleWorkbench 删除在途行的 busy 判定（PR #92 复审 Important）", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("详情未加载（生命周期待查看组）的行：DELETE 在途期间 aria-busy、按钮禁用，连点不重复发 DELETE", async () => {
    const harness = deletePendingGateFetch();
    vi.stubGlobal("fetch", harness.fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const pendingRowSelect = await within(queueRegion()).findByRole("button", {
      name: "选择 Issue 详情在途议题",
    });
    expect(pendingRowSelect).toBeEnabled();
    // 详情读取被挂起：行停留在「生命周期待查看」组，而非任何已加载分组。
    expect(rowGroupKey("issue_0001")).toBe("unloaded");

    const deleteButton = within(queueRegion()).getByRole("button", {
      name: "删除 Issue 详情在途议题",
    });
    expect(queueRow("issue_0001")).not.toHaveAttribute("aria-busy");
    expect(deleteButton).toBeEnabled();

    await user.click(deleteButton);
    expect(harness.deleteRequestCount("issue_0001")).toBe(1);

    // DELETE 在途：行进入 busy、删除按钮禁用。修复前 deletingIssueId 只能从
    // 已加载卡片反查，未加载行拿不到，busy 不出现、按钮保持可用（连点可再发
    // 一个 DELETE）。此断言即红测试锚点。
    await waitFor(() =>
      expect(queueRow("issue_0001")).toHaveAttribute("aria-busy", "true"),
    );
    expect(deleteButton).toBeDisabled();

    // 连点被禁用按钮挡住：不产生第二个 DELETE。
    await user.click(deleteButton);
    expect(harness.deleteRequestCount("issue_0001")).toBe(1);

    // 结算 DELETE：行按删除回执从队列消失。
    await act(async () => {
      harness.releaseDelete("issue_0001");
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "选择 Issue 详情在途议题" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("详情已加载的行保持既有删除 busy 表现", async () => {
    const harness = deletePendingGateFetch();
    vi.stubGlobal("fetch", harness.fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await within(queueRegion()).findByRole("button", {
      name: "选择 Issue 详情已就绪议题",
    });
    expect(rowGroupKey("issue_0002")).not.toBe("unloaded");

    const deleteButton = within(queueRegion()).getByRole("button", {
      name: "删除 Issue 详情已就绪议题",
    });
    expect(deleteButton).toBeEnabled();

    await user.click(deleteButton);
    expect(harness.deleteRequestCount("issue_0002")).toBe(1);

    await waitFor(() =>
      expect(queueRow("issue_0002")).toHaveAttribute("aria-busy", "true"),
    );
    expect(deleteButton).toBeDisabled();
    await user.click(deleteButton);
    expect(harness.deleteRequestCount("issue_0002")).toBe(1);

    await act(async () => {
      harness.releaseDelete("issue_0002");
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "选择 Issue 详情已就绪议题" }),
      ).not.toBeInTheDocument(),
    );
  });
});
