// PR #92 四审 Important 回归（在途删除守卫）：在途删除的 UI 锁（deletingCardKeys）
// 在切换 Project 时被 resetProjectScopeState 清空——在 Project A 发起 Issue X 的
// DELETE 后切到 B 再切回 A，若 X 的 DELETE 仍在途且列表仍显示 X，其删除按钮会
// 重新可用，再点即发出第二个 DELETE /api/projects/<id>/issues/X（第二次 issue
// not found 并留下可见错误，尽管第一次已成功）。守卫必须挂在请求发出处（hook
// 内 ref 在途集合），与 UI 状态重置无关：同一 (Project, Issue) 的 DELETE 未结算
// 前不出站第二个请求，结算（成功或失败）即释放；请求形状与错误文案不变。
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProductIssueListResponse } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponseValue,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
};

const ALPHA = "project_0001";
const BETA = "project_0002";
const ALPHA_ISSUE_ID = "issue_0001";
const ALPHA_ISSUE_TITLE = "跨项目在途议题";
const BETA_ISSUE_TITLE = "Beta 普通议题";

// 两个 Project（Alpha/Beta）；Alpha 的 issue_0001 详情读取永久挂起（行停留
// 「生命周期待查看」组）。issue DELETE 一律挂起、由测试按 (projectId, issueId)
// FIFO 结算并计数出站次数；成功结算后 issues 列表剔除该行，restoreIssue 模拟
// 服务端复用同 ID 重新下发记录。
function deleteGuardGateFetch() {
  const baseFetch = lifecycleFetch({
    projects: [projectRecord(ALPHA, "Alpha"), projectRecord(BETA, "Beta")],
    issueTitlesByProject: {
      [ALPHA]: ALPHA_ISSUE_TITLE,
      [BETA]: BETA_ISSUE_TITLE,
    },
  });
  const detailGate = deferred<Response>();
  const deletedIssueKeys = new Set<string>();
  const deleteGates = new Map<string, Deferred<Response>[]>();
  const deleteRequests = new Map<string, number>();

  function takeDeleteGate(key: string): Deferred<Response> {
    const gate = deleteGates.get(key)?.shift();
    if (!gate) {
      throw new Error(`no in-flight DELETE for ${key}`);
    }
    return gate;
  }

  const fetchMock = vi.fn(
    async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const url = String(input);
      const lifecycleMatch = url.match(
        /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/,
      );
      if (lifecycleMatch && lifecycleMatch[1] === ALPHA_ISSUE_ID) {
        return detailGate.promise;
      }
      const issueDeleteMatch = url.match(
        /^\/api\/projects\/([^/]+)\/issues\/([^/?]+)$/,
      );
      if (issueDeleteMatch && init?.method === "DELETE") {
        const key = `${issueDeleteMatch[1]}/${issueDeleteMatch[2]}`;
        deleteRequests.set(key, (deleteRequests.get(key) ?? 0) + 1);
        const queue = deleteGates.get(key) ?? [];
        queue.push(deferred<Response>());
        deleteGates.set(key, queue);
        return queue[queue.length - 1].promise;
      }
      const issuesMatch = url.match(/^\/api\/projects\/([^/]+)\/issues$/);
      if (issuesMatch) {
        const response = await baseFetch(input, init);
        const data = (await response.json()) as ProductIssueListResponse;
        return jsonResponseValue({
          issues: data.issues.filter(
            (issue) =>
              !deletedIssueKeys.has(`${issuesMatch[1]}/${issue.issue_id}`),
          ),
        });
      }
      return baseFetch(input, init);
    },
  );

  return {
    fetchMock,
    deleteRequestCount: (projectId: string, issueId: string) =>
      deleteRequests.get(`${projectId}/${issueId}`) ?? 0,
    // 结算最早的在途 DELETE（成功）：后续 issues 列表读取剔除该行。
    releaseDelete: (projectId: string, issueId: string) => {
      const key = `${projectId}/${issueId}`;
      deletedIssueKeys.add(key);
      takeDeleteGate(key).resolve(jsonResponseValue({ status: "deleted" }));
    },
    // 结算最早的在途 DELETE（失败）：行保留，错误文案沿用 reason.message。
    rejectDelete: (projectId: string, issueId: string, message: string) => {
      takeDeleteGate(`${projectId}/${issueId}`).reject(new Error(message));
    },
    // 模拟服务端复用已删除 ID：下一次列表读取重新下发该行。
    restoreIssue: (projectId: string, issueId: string) => {
      deletedIssueKeys.delete(`${projectId}/${issueId}`);
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

describe("IssueLifecycleWorkbench 删除在途守卫（PR #92 四审 Important）", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("DELETE 在途期间切走 Project 再切回（UI pending 集已清空）：再点不发出第二个 DELETE", async () => {
    const harness = deleteGuardGateFetch();
    vi.stubGlobal("fetch", harness.fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await within(queueRegion()).findByRole("button", {
      name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    // 详情读取被挂起：行停留「生命周期待查看」组（not-yet-loaded 前提）。
    expect(rowGroupKey(ALPHA_ISSUE_ID)).toBe("unloaded");

    const deleteButton = within(queueRegion()).getByRole("button", {
      name: `删除 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    await user.click(deleteButton);
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(1);

    // 切到 Beta：resetProjectScopeState 清空 deletingCardKeys（UI 锁失效源）。
    const beta = screen.getByRole("button", { name: "Beta" });
    await waitFor(() => expect(beta).toBeEnabled());
    await user.click(beta);
    await within(queueRegion()).findByRole("button", {
      name: `选择 Issue ${BETA_ISSUE_TITLE}`,
    });

    // 切回 Alpha：DELETE 未结算，issue_0001 仍在服务端列表，行重新渲染。
    const alpha = screen.getByRole("button", { name: "Alpha" });
    await waitFor(() => expect(alpha).toBeEnabled());
    await user.click(alpha);
    await within(queueRegion()).findByRole("button", {
      name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
    });

    // 复审缺陷前提：UI 层 pending 集已被项目切换清空——行不 busy、删除按钮
    // 重新可用。此时点击不得发出第二个 DELETE（守卫在请求发出处，而非 UI
    // 状态）。修复前此处会发出第二个 DELETE，本断言即红测试锚点。
    expect(queueRow(ALPHA_ISSUE_ID)).not.toHaveAttribute("aria-busy");
    const reappearedDelete = within(queueRegion()).getByRole("button", {
      name: `删除 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    expect(reappearedDelete).toBeEnabled();
    await user.click(reappearedDelete);
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(1);

    // 结算第一个 DELETE：行按删除回执消失，全程只有一个出站请求。
    await act(async () => {
      harness.releaseDelete(ALPHA, ALPHA_ISSUE_ID);
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
        }),
      ).not.toBeInTheDocument(),
    );
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(1);
  });

  it("首个 DELETE 成功结算后守卫释放：同一 Issue 后续动作可再次删除", async () => {
    const harness = deleteGuardGateFetch();
    vi.stubGlobal("fetch", harness.fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await within(queueRegion()).findByRole("button", {
      name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    await user.click(
      within(queueRegion()).getByRole("button", {
        name: `删除 Issue ${ALPHA_ISSUE_TITLE}`,
      }),
    );
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(1);

    // 成功结算：行按删除回执消失。
    await act(async () => {
      harness.releaseDelete(ALPHA, ALPHA_ISSUE_ID);
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
        }),
      ).not.toBeInTheDocument(),
    );

    // 服务端可复用已删除 ID：刷新后同 ID 记录重新出现在队列，删除按钮可用。
    harness.restoreIssue(ALPHA, ALPHA_ISSUE_ID);
    await user.click(screen.getByRole("button", { name: "刷新" }));
    const reappearedDelete = await within(queueRegion()).findByRole("button", {
      name: `删除 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    expect(reappearedDelete).toBeEnabled();

    // 守卫已随首个 DELETE 结算释放：再次删除正常出站并结算成功。
    await user.click(reappearedDelete);
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(2);
    await act(async () => {
      harness.releaseDelete(ALPHA, ALPHA_ISSUE_ID);
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
        }),
      ).not.toBeInTheDocument(),
    );
  });

  it("DELETE 失败结算后守卫释放且错误文案不变：可再次删除", async () => {
    const harness = deleteGuardGateFetch();
    vi.stubGlobal("fetch", harness.fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await within(queueRegion()).findByRole("button", {
      name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    await user.click(
      within(queueRegion()).getByRole("button", {
        name: `删除 Issue ${ALPHA_ISSUE_TITLE}`,
      }),
    );
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(1);

    // 失败结算：行保留，错误文案沿用 reason.message（errorMessage 契约不变）。
    await act(async () => {
      harness.rejectDelete(ALPHA, ALPHA_ISSUE_ID, "删除被服务端拒绝");
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "删除被服务端拒绝",
    );
    const retryDelete = within(queueRegion()).getByRole("button", {
      name: `删除 Issue ${ALPHA_ISSUE_TITLE}`,
    });
    expect(retryDelete).toBeEnabled();

    // 守卫已随失败结算释放：再次删除正常出站并结算成功。
    await user.click(retryDelete);
    expect(harness.deleteRequestCount(ALPHA, ALPHA_ISSUE_ID)).toBe(2);
    await act(async () => {
      harness.releaseDelete(ALPHA, ALPHA_ISSUE_ID);
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: `选择 Issue ${ALPHA_ISSUE_TITLE}`,
        }),
      ).not.toBeInTheDocument(),
    );
  });
});
