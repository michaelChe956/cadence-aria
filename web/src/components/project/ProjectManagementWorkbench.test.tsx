import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProjectManagementWorkbench } from "./ProjectManagementWorkbench";

describe("ProjectManagementWorkbench", () => {
  it("loads projects and legacy issues then opens executable issue context", async () => {
    const fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === "/api/projects") {
        return jsonResponse({
          projects: [
            {
              project_id: "project_0001",
              name: "Aria 项目",
              description: "项目管理工作台",
              created_at: "2026-05-14T00:00:00Z",
              updated_at: "2026-05-14T00:00:00Z",
              last_opened_at: null,
            },
          ],
        });
      }
      if (url === "/api/issues") {
        return jsonResponse({
          issues: [
            {
              issue_id: "issue_0001",
              title: "实现项目切换",
              description: "连接项目与 issue 队列",
              status: "in_progress",
              workspace_id: "workspace_0001",
              task_id: "task_0001",
              session_id: "session_0001",
              change_id: "project-switcher",
              created_at: "2026-05-14T00:00:00Z",
              updated_at: "2026-05-14T00:00:00Z",
            },
          ],
        });
      }
      return jsonResponse({});
    });
    vi.stubGlobal("fetch", fetchSpy);
    const onOpenExecution = vi.fn();

    render(<ProjectManagementWorkbench onOpenExecution={onOpenExecution} />);

    expect(await screen.findByRole("combobox", { name: "选择项目" })).toHaveDisplayValue(
      "Aria 项目",
    );
    expect(
      await within(screen.getByRole("region", { name: "Issue 列表面板" })).findByText(
        "实现项目切换",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "实现项目切换" })).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "仓库面板" })).getByText("Aria 项目"))
      .toBeInTheDocument();
    await waitFor(() => expect(fetchSpy).toHaveBeenCalledWith("/api/projects", expect.anything()));
    expect(fetchSpy).toHaveBeenCalledWith("/api/issues", expect.anything());

    await userEvent.click(screen.getByRole("button", { name: "打开执行" }));

    expect(onOpenExecution).toHaveBeenCalledWith({
      issueId: "issue_0001",
      workspaceId: "workspace_0001",
      taskId: "task_0001",
    });
  });
});

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}
