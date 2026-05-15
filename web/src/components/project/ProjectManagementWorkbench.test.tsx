import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProjectManagementWorkbench } from "./ProjectManagementWorkbench";

describe("ProjectManagementWorkbench", () => {
  it("renders the project workbench as a dense three-zone tool surface", async () => {
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

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    expect(await screen.findByRole("banner")).toHaveTextContent("Aria Web");
    expect(screen.getByRole("main", { name: "项目管理工作台" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Issue 列表面板" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Issue 详情" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "仓库面板" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Provider execution panel" })).toBeInTheDocument();
    expect(screen.getByText("Repo path")).toBeInTheDocument();
    expect(screen.getByText("Repo hash")).toBeInTheDocument();
    expect(screen.getByText("Runtime root")).toBeInTheDocument();
    expect(screen.queryByText("playful coding workbench")).not.toBeInTheDocument();
  });

  it("surfaces gate actions under blocked issue detail", async () => {
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
              issue_id: "issue_blocked_0001",
              title: "等待人工确认",
              description: "Provider gate 已暂停",
              status: "blocked",
              workspace_id: "workspace_0001",
              task_id: "task_0001",
              session_id: "session_0001",
              change_id: "blocked-gate",
              created_at: "2026-05-14T00:00:00Z",
              updated_at: "2026-05-14T00:00:00Z",
            },
          ],
        });
      }
      return jsonResponse({});
    });
    vi.stubGlobal("fetch", fetchSpy);

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    const gate = await screen.findByRole("region", { name: "Gate action bar" });
    expect(within(gate).getByRole("button", { name: "确认继续" })).toBeInTheDocument();
    expect(within(gate).getByRole("button", { name: "要求修改" })).toBeInTheDocument();
    expect(within(gate).getByRole("button", { name: "终止" })).toBeInTheDocument();
  });

  it("marks completed issues as acceptance in the phase rail", async () => {
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
              issue_id: "issue_done_0001",
              title: "完成验收",
              description: "生命周期应进入验收阶段",
              status: "completed",
              workspace_id: "workspace_0001",
              task_id: "task_0001",
              session_id: "session_0001",
              change_id: "acceptance-check",
              created_at: "2026-05-14T00:00:00Z",
              updated_at: "2026-05-14T00:00:00Z",
            },
          ],
        });
      }
      return jsonResponse({});
    });
    vi.stubGlobal("fetch", fetchSpy);

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    const phaseRail = await screen.findByRole("navigation", { name: "Issue 阶段" });
    expect(within(phaseRail).getByText("Acceptance").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
  });

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
