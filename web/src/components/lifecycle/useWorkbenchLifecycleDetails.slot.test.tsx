import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProductIssue } from "../../api/types";
import { useWorkbenchLifecycleDetails } from "./useWorkbenchLifecycleDetails";

// 回归：项目切换后详情批次的并发槽饥饿（浏览器取证：上一项目 3 个慢读
// 未返回时切项目，新项目 0 个详情读发出，详情面板停在「正在加载生命周期…」）。
// 要求：新批次可自带并发预算发读；旧批次迟到响应仍被丢弃；
// 当前批次在飞读数不超过 DETAIL_CONCURRENCY（3）。

const DETAIL_CONCURRENCY = 3;

function issueFixture(issueId: string, projectId: string): ProductIssue {
  return {
    issue_id: issueId,
    project_id: projectId,
    repo_id: null,
    base_branch: null,
    workspace_id: null,
    task_id: null,
    session_id: null,
    title: `Issue ${issueId}`,
    description: null,
    change_id: `${issueId}-change`,
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  };
}

function lifecycleBody(issue: ProductIssue) {
  return {
    issue,
    story_specs: [],
    design_specs: [],
    work_item_plans: [],
    work_items: [],
    work_item_repository_groups: [],
    workspace_sessions: [],
    coding_attempts: [],
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

type HangingRead = {
  resolve: (body: unknown) => void;
  reject: (reason?: unknown) => void;
};

function lifecycleUrl(issueId: string, projectId: string) {
  return `/api/issues/${issueId}/lifecycle?project_id=${projectId}`;
}

function installDetailFetch(hangingIssueIds: string[]) {
  const lifecycleUrls: string[] = [];
  const hanging = new Map<string, HangingRead>();
  for (const issueId of hangingIssueIds) {
    hanging.set(issueId, {
      resolve: () => {},
      reject: () => {},
    });
  }
  const fetchMock = vi.fn((input: RequestInfo | URL) => {
    const url = String(input);
    const match = url.match(
      /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/,
    );
    if (!match) {
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }
    lifecycleUrls.push(url);
    const [, issueId, projectId] = match;
    const deferred = hanging.get(issueId);
    if (deferred) {
      return new Promise<Response>((resolve, reject) => {
        deferred.resolve = (body: unknown) => resolve(jsonResponse(body));
        deferred.reject = (reason?: unknown) => reject(reason);
      });
    }
    return Promise.resolve(
      jsonResponse(lifecycleBody(issueFixture(issueId, projectId))),
    );
  });
  vi.stubGlobal("fetch", fetchMock);
  return { lifecycleUrls, hanging };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useWorkbenchLifecycleDetails 批次槽饥饿回归", () => {
  it("切换项目后新批次立即发读并落位，旧项目迟到响应不覆盖新状态", async () => {
    const { lifecycleUrls, hanging } = installDetailFetch([
      "issue_A1",
      "issue_A2",
      "issue_A3",
    ]);
    const { result } = renderHook(() => useWorkbenchLifecycleDetails());

    const projectAIssues = ["issue_A1", "issue_A2", "issue_A3"].map(
      (issueId) => issueFixture(issueId, "project_A"),
    );
    act(() => {
      result.current.hydrateDetails(
        "project_A",
        projectAIssues,
        "issue_A1",
        undefined,
      );
    });
    // 项目 A 的 3 个读全部发出且悬挂（模拟浏览器取证中的慢读）。
    expect(lifecycleUrls).toHaveLength(DETAIL_CONCURRENCY);

    const projectBIssue = issueFixture("issue_B1", "project_B");
    act(() => {
      result.current.hydrateDetails(
        "project_B",
        [projectBIssue],
        "issue_B1",
        undefined,
      );
    });

    // 新批次不受旧批次在飞读占用影响：project_B 的读已发出。
    expect(lifecycleUrls).toContain(lifecycleUrl("issue_B1", "project_B"));
    await waitFor(() => {
      expect(result.current.details.get("issue_B1")?.issue.project_id).toBe(
        "project_B",
      );
    });

    // 旧项目读迟到返回：批次身份守卫必须丢弃，不写回 details/errors。
    await act(async () => {
      hanging.get("issue_A1")?.resolve(
        lifecycleBody(issueFixture("issue_A1", "project_A")),
      );
      hanging.get("issue_A2")?.resolve(
        lifecycleBody(issueFixture("issue_A2", "project_A")),
      );
      hanging.get("issue_A3")?.resolve(
        lifecycleBody(issueFixture("issue_A3", "project_A")),
      );
    });
    expect(result.current.details.has("issue_A1")).toBe(false);
    expect(result.current.details.size).toBe(1);
    expect(result.current.detailErrors.size).toBe(0);
  });

  it("当前批次在飞读不超过 DETAIL_CONCURRENCY，槽位随读完成释放", async () => {
    const { lifecycleUrls, hanging } = installDetailFetch([
      "issue_A1",
      "issue_A2",
      "issue_A3",
      "issue_B1",
      "issue_B2",
      "issue_B3",
      "issue_B4",
    ]);
    const { result } = renderHook(() => useWorkbenchLifecycleDetails());

    act(() => {
      result.current.hydrateDetails(
        "project_A",
        ["issue_A1", "issue_A2", "issue_A3"].map((issueId) =>
          issueFixture(issueId, "project_A"),
        ),
        "issue_A1",
        undefined,
      );
    });
    expect(lifecycleUrls).toHaveLength(DETAIL_CONCURRENCY);

    const projectBIssues = ["issue_B1", "issue_B2", "issue_B3", "issue_B4"].map(
      (issueId) => issueFixture(issueId, "project_B"),
    );
    act(() => {
      result.current.hydrateDetails(
        "project_B",
        projectBIssues,
        "issue_B1",
        undefined,
      );
    });
    const projectBUrls = lifecycleUrls.filter((url) =>
      url.includes("project_id=project_B"),
    );
    // 新批次同样只发 3 个：B1（焦点）+ B2 + B3，B4 仍在队列。
    expect(projectBUrls).toEqual([
      lifecycleUrl("issue_B1", "project_B"),
      lifecycleUrl("issue_B2", "project_B"),
      lifecycleUrl("issue_B3", "project_B"),
    ]);

    // B1 完成释放槽位后，B4 才被派发；B1 的结果正常落位。
    await act(async () => {
      hanging.get("issue_B1")?.resolve(
        lifecycleBody(issueFixture("issue_B1", "project_B")),
      );
    });
    await waitFor(() => {
      expect(lifecycleUrls).toContain(lifecycleUrl("issue_B4", "project_B"));
    });
    expect(result.current.details.get("issue_B1")?.issue.project_id).toBe(
      "project_B",
    );
  });

  it("clearDetails 丢弃旧批次后，迟到的旧读不产生任何状态写入", async () => {
    const { lifecycleUrls, hanging } = installDetailFetch([
      "issue_A1",
      "issue_A2",
      "issue_A3",
    ]);
    const { result } = renderHook(() => useWorkbenchLifecycleDetails());

    act(() => {
      result.current.hydrateDetails(
        "project_A",
        ["issue_A1", "issue_A2", "issue_A3"].map((issueId) =>
          issueFixture(issueId, "project_A"),
        ),
        "issue_A1",
        undefined,
      );
    });
    expect(lifecycleUrls).toHaveLength(DETAIL_CONCURRENCY);

    act(() => {
      result.current.clearDetails();
    });
    expect(result.current.details.size).toBe(0);

    await act(async () => {
      hanging.get("issue_A1")?.resolve(
        lifecycleBody(issueFixture("issue_A1", "project_A")),
      );
      hanging.get("issue_A2")?.reject(new Error("late failure"));
      hanging.get("issue_A3")?.resolve(
        lifecycleBody(issueFixture("issue_A3", "project_A")),
      );
    });
    expect(result.current.details.size).toBe(0);
    expect(result.current.detailErrors.size).toBe(0);
  });
});
