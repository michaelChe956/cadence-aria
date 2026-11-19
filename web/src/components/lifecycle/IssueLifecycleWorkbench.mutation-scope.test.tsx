import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { IssueLifecycleResponse, ProductIssueListResponse } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  initialLifecycleData,
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock(import("./IssueLifecycleWorkbenchParts"), async (importOriginal) => ({
  ...(await importOriginal()),
  waitForDeleteExitAnimation: async () => {},
}));

const DELETE_STORY = "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001";
const ALPHA_DETAIL = "/api/issues/issue_0001/lifecycle?project_id=project_0001";
const BETA_ISSUES = "/api/projects/project_0002/issues";
const BETA_DETAIL = "/api/issues/issue_0001/lifecycle?project_id=project_0002";
const DELETE_ISSUE = "/api/projects/project_0001/issues/issue_0001";
const PROJECTS = "/api/projects";
const DELETE_BUTTON = "删除 Story Spec 会话过期提示";
const BETA_STORY_TITLE = "Beta 移动端刷新需求";

describe("IssueLifecycleWorkbench mutation refresh", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("keeps the new project selected when a deletion started in another project completes", async () => {
    const pendingDelete = deferred<Response>();
    const baseFetch = lifecycleFetch({
      projects: [projectRecord("project_0001", "Alpha"), projectRecord("project_0002", "Beta")],
      issueTitlesByProject: { project_0001: "Alpha issue", project_0002: "Beta issue" },
    });
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === DELETE_STORY && init?.method === "DELETE") return pendingDelete.promise;
      if (path === BETA_ISSUES) {
        const data: ProductIssueListResponse = await (await baseFetch(input, init)).json();
        return jsonResponse({ issues: data.issues.map((issue) => ({ ...issue, issue_id: "issue_0001" })) });
      }
      if (path === BETA_DETAIL) {
        // Beta 的同 ID Story 走真实 lifecycle 夹具载荷（story_spec_0001 与 Alpha
        // 撞 ID），标题/预览换成 Beta 专属文案以区分抽屉内容归属。
        const data = initialLifecycleData(
          "issue_0001",
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
        );
        return jsonResponse({
          issue: {
            issue_id: "issue_0001",
            project_id: "project_0002",
            repo_id: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: "Beta issue",
            description: "描述",
            change_id: "mobile-refresh",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
          story_specs: [
            {
              ...data.story_specs[0],
              title: BETA_STORY_TITLE,
              current_markdown_preview: "## 功能需求\n\n[REQ-BETA] 移动端刷新需求。",
            },
          ],
          design_specs: data.design_specs,
          work_item_plans: data.work_item_plans,
          work_items: data.work_items,
          work_item_repository_groups: data.work_item_repository_groups,
          workspace_sessions: data.workspace_sessions,
          coding_attempts: data.coding_attempts,
        });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    const detail = screen.getByRole("region", { name: "Issue 生命周期详情" });

    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(within(detail).getByRole("button", { name: DELETE_BUTTON }));
    expect(fetchMock).toHaveBeenCalledWith(DELETE_STORY, expect.objectContaining({ method: "DELETE" }));
    const beta = screen.getByRole("button", { name: "Beta" });
    await waitFor(() => expect(beta).toBeEnabled());
    await user.click(beta);
    await within(detail).findByRole("heading", { name: "Beta issue" });
    await within(detail).findByTestId("stage-tab-story");
    await waitFor(() => expect(beta).toBeEnabled());

    // 释放 Alpha 删除响应前，先打开 Beta 同 ID Story（story_spec_0001 撞 ID）的抽屉。
    await user.click(screen.getByTestId("stage-tab-story"));
    const storyRegion = screen.getByRole("region", { name: "Story Spec 内容" });
    await user.click(within(storyRegion).getByRole("button", { name: BETA_STORY_TITLE }));
    const drawer = await screen.findByTestId("lifecycle-card-drawer");
    expect(drawer).toHaveTextContent(BETA_STORY_TITLE);

    await act(async () => {
      pendingDelete.resolve(await baseFetch(DELETE_STORY, { method: "DELETE" }));
    });
    expect(screen.getByRole("button", { name: "Beta" })).toHaveAttribute("aria-pressed", "true");
    expect(within(detail).getByRole("heading", { name: "Beta issue" })).toBeVisible();
    // 迟到的 Alpha 删除不得关掉 Beta 的同 ID 抽屉，内容仍是 Beta 专属。
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(BETA_STORY_TITLE);
  });

  it("does not restore a deleted story while its details are reloading or after a read failure", async () => {
    const pendingRead = deferred<Response>();
    const baseFetch = lifecycleFetch();
    let deleted = false;
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === DELETE_STORY && init?.method === "DELETE") {
        const response = await baseFetch(input, init);
        deleted = response.ok;
        return response;
      }
      if (path === ALPHA_DETAIL && deleted) return pendingRead.promise;
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    const detail = screen.getByRole("region", { name: "Issue 生命周期详情" });

    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(within(detail).getByRole("button", { name: DELETE_BUTTON }));
    await waitFor(() => expect(fetchMock.mock.calls.filter(([path]) => String(path) === ALPHA_DETAIL)).toHaveLength(2));
    expect(within(detail).queryByRole("button", { name: "会话过期提示" })).not.toBeInTheDocument();
    expect(within(detail).queryByRole("button", { name: DELETE_BUTTON })).not.toBeInTheDocument();

    await act(async () => {
      pendingRead.resolve(new Response(JSON.stringify({ message: "回读失败" }), { status: 503 }));
    });
    expect(within(detail).getByRole("alert")).toHaveTextContent("回读失败");
    expect(within(detail).queryByRole("button", { name: "会话过期提示" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "选择 Issue 登录会话过期" })).toBeEnabled();
  });
  it("does not leave a deleted issue selectable after the navigation it raced against fails", async () => {
    const pendingDelete = deferred<Response>();
    const pendingNavigation = deferred<Response>();
    const baseFetch = lifecycleFetch({
      projects: [projectRecord("project_0001", "Alpha"), projectRecord("project_0002", "Beta")],
      issueTitlesByProject: { project_0001: "Alpha issue", project_0002: "Beta issue" },
    });
    const original: IssueLifecycleResponse = await (await baseFetch(ALPHA_DETAIL)).json();
    let replacement: IssueLifecycleResponse | undefined;
    let projectReads = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === DELETE_ISSUE && init?.method === "DELETE") return pendingDelete.promise;
      if (path === PROJECTS) {
        projectReads += 1;
        if (projectReads === 2) return pendingNavigation.promise;
      }
      if (replacement && path === ALPHA_DETAIL) return jsonResponse(replacement);
      if (replacement && path === "/api/projects/project_0001/issues") {
        return jsonResponse({ issues: [replacement.issue] });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "删除 Issue Alpha issue" }));
    expect(fetchMock).toHaveBeenCalledWith(DELETE_ISSUE, expect.objectContaining({ method: "DELETE" }));
    const beta = screen.getByRole("button", { name: "Beta" });
    await waitFor(() => expect(beta).toBeEnabled());
    await user.click(beta);
    await waitFor(() =>
      expect(fetchMock.mock.calls.filter(([path]) => String(path) === PROJECTS)).toHaveLength(2),
    );

    await act(async () => {
      pendingDelete.resolve(await baseFetch(DELETE_ISSUE, { method: "DELETE" }));
    });
    // Beta 导航的项目目录读取失败：选中态留在 Alpha，且不得把已删除的 Issue 行留在队列里。
    await act(async () => {
      pendingNavigation.reject(new Error("项目目录读取失败"));
    });

    expect(screen.getByRole("button", { name: "Alpha" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("alert")).toHaveTextContent("项目目录读取失败");
    expect(screen.queryByRole("button", { name: "选择 Issue Alpha issue" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "删除 Issue Alpha issue" })).not.toBeInTheDocument();

    // The server can reuse a deleted issue ID. A fresh read must show the new record.
    replacement = { ...original, issue: { ...original.issue, title: "Replacement Alpha issue" } };
    await user.click(screen.getByRole("button", { name: "刷新" }));
    expect(await screen.findByRole("button", { name: "选择 Issue Replacement Alpha issue" })).toBeEnabled();
  });
});