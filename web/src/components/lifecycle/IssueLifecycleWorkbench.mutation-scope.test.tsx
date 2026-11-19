import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProductIssueListResponse } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
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
const DELETE_BUTTON = "删除 Story Spec 会话过期提示";

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

    await act(async () => {
      pendingDelete.resolve(await baseFetch(DELETE_STORY, { method: "DELETE" }));
    });
    expect(screen.getByRole("button", { name: "Beta" })).toHaveAttribute("aria-pressed", "true");
    expect(within(detail).getByRole("heading", { name: "Beta issue" })).toBeVisible();
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
});