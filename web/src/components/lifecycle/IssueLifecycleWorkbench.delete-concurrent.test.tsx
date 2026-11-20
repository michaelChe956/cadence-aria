// PR #92 三审 Important 回归（并发删除）：在途删除此前只记一个 deletingCardKey，
// 先删未加载 Issue A 再删 Issue B 时键被 B 覆盖——A 的行丢失 busy、删除按钮重新
// 可用，再点 A 会发出第二个 DELETE /api/projects/<id>/issues/<issueId>（第二次
// issue not found 并留下可见错误）。契约：删除在途状态是一整个集合，A、B 同时
// 在途期间两行都 aria-busy、删除按钮都禁用，连点不产生第二个 DELETE；先结算的
// 一个不解除另一个的 busy。请求形状、确认流、错误文案均不变。
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

// 两个 Issue 的 lifecycle 读取都永久挂起（两行都停留 unloaded 组），DELETE 一律
// 挂起、由测试按需结算并按 issueId 计数出站次数。
function concurrentDeleteGateFetch() {
  const heldDetailIssueIds = new Set(["issue_0001", "issue_0002"]);
  const deletedIssueIds = new Set<string>();
  const issues = [
    issueRecord("issue_0001", "并发在途议题甲"),
    issueRecord("issue_0002", "并发在途议题乙"),
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
      if (lifecycleMatch && heldDetailIssueIds.has(lifecycleMatch[1])) {
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

describe("IssueLifecycleWorkbench 并发删除（PR #92 三审 Important）", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("先删 A 再删 B：两行 DELETE 在途期间同时 busy、按钮禁用，连点 A 不再发第二个 DELETE", async () => {
    const harness = concurrentDeleteGateFetch();
    vi.stubGlobal("fetch", harness.fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const selectA = await within(queueRegion()).findByRole("button", {
      name: "选择 Issue 并发在途议题甲",
    });
    const selectB = await within(queueRegion()).findByRole("button", {
      name: "选择 Issue 并发在途议题乙",
    });
    expect(selectA).toBeEnabled();
    expect(selectB).toBeEnabled();
    // 两行详情都在途：都停留在「生命周期待查看」组（not-yet-loaded 前提）。
    expect(
      queueRow("issue_0001").closest('[data-testid="issue-queue-group-rows"]'),
    ).toHaveAttribute("data-group-key", "unloaded");
    expect(
      queueRow("issue_0002").closest('[data-testid="issue-queue-group-rows"]'),
    ).toHaveAttribute("data-group-key", "unloaded");

    const deleteA = within(queueRegion()).getByRole("button", {
      name: "删除 Issue 并发在途议题甲",
    });
    const deleteB = within(queueRegion()).getByRole("button", {
      name: "删除 Issue 并发在途议题乙",
    });

    // 先删 A，再删 B：两个 DELETE 同时在途。
    await user.click(deleteA);
    expect(harness.deleteRequestCount("issue_0001")).toBe(1);
    await user.click(deleteB);
    expect(harness.deleteRequestCount("issue_0002")).toBe(1);

    // 契约：B 发起后 A 的行不得退出 busy、删除按钮不得重新可用（修复前单键
    // deletingCardKey 被 B 覆盖，A 行 busy 丢失）。此断言即红测试锚点。
    await waitFor(() =>
      expect(queueRow("issue_0001")).toHaveAttribute("aria-busy", "true"),
    );
    await waitFor(() =>
      expect(queueRow("issue_0002")).toHaveAttribute("aria-busy", "true"),
    );
    expect(deleteA).toBeDisabled();
    expect(deleteB).toBeDisabled();

    // 连点被禁用按钮挡住：A 不产生第二个 DELETE。
    await user.click(deleteA);
    expect(harness.deleteRequestCount("issue_0001")).toBe(1);
    expect(harness.deleteRequestCount("issue_0002")).toBe(1);

    // 先结算 B：B 行按删除回执消失，A 仍在途、行保持 busy（集合未被 B 清空）。
    await act(async () => {
      harness.releaseDelete("issue_0002");
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "选择 Issue 并发在途议题乙" }),
      ).not.toBeInTheDocument(),
    );
    expect(queueRow("issue_0001")).toHaveAttribute("aria-busy", "true");
    expect(deleteA).toBeDisabled();

    // 再结算 A：A 行消失。
    await act(async () => {
      harness.releaseDelete("issue_0001");
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "选择 Issue 并发在途议题甲" }),
      ).not.toBeInTheDocument(),
    );
  });
});
