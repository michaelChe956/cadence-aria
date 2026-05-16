import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProjectManagementWorkbench } from "./ProjectManagementWorkbench";

describe("ProjectManagementWorkbench", () => {
  it("renders the issue lifecycle workbench wrapper", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<ProjectManagementWorkbench onOpenExecution={vi.fn()} />);

    expect(
      await screen.findByRole("main", { name: "Issue 生命周期工作台" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Issue 列" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Story Spec 列" })).toHaveTextContent(
      "会话过期提示",
    );
    expect(screen.getByRole("region", { name: "Design Spec 列" })).toHaveTextContent(
      "前端提示设计",
    );
    expect(screen.getByRole("region", { name: "Work Item 列" })).toHaveTextContent(
      "实现提示组件",
    );
  });
});

function lifecycleFetch() {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/projects") {
      return jsonResponse({
        projects: [
          {
            project_id: "project_0001",
            name: "Aria",
            description: null,
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
            last_opened_at: null,
          },
        ],
      });
    }
    if (url === "/api/projects/project_0001/repositories") {
      return jsonResponse({
        repositories: [
          {
            repository_id: "repository_0001",
            project_id: "project_0001",
            name: "Aria Repo",
            path: "/tmp/aria",
            repo_hash: "hash",
            runtime_root: "/tmp/aria/.aria/runtime",
            default_policy_preset: "manual-write",
            default_provider_mode: "fake",
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    if (url === "/api/projects/project_0001/issues") {
      return jsonResponse({
        issues: [
          {
            issue_id: "issue_0001",
            project_id: "project_0001",
            repo_id: "repository_0001",
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: "登录会话过期",
            description: "描述",
            change_id: "login-session-expired",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    if (url === "/api/issues/issue_0001/lifecycle?project_id=project_0001") {
      return jsonResponse({
        issue: {
          issue_id: "issue_0001",
          project_id: "project_0001",
          repo_id: "repository_0001",
          workspace_id: null,
          task_id: null,
          session_id: null,
          title: "登录会话过期",
          description: "描述",
          change_id: "login-session-expired",
          phase: "clarification",
          status: "draft",
          active_binding_id: null,
          artifacts: [],
          created_at: "2026-05-16T00:00:00Z",
          updated_at: "2026-05-16T00:00:00Z",
        },
        story_specs: [
          {
            story_spec_id: "story_spec_0001",
            issue_id: "issue_0001",
            repository_id: "repository_0001",
            title: "会话过期提示",
            current_version: 1,
            confirmation_status: "confirmed",
          },
        ],
        design_specs: [
          {
            design_spec_id: "design_spec_0001",
            issue_id: "issue_0001",
            story_spec_ids: ["story_spec_0001"],
            design_kind: "frontend",
            title: "前端提示设计",
            current_version: 1,
            confirmation_status: "confirmed",
          },
        ],
        work_items: [
          {
            work_item_id: "work_item_0001",
            issue_id: "issue_0001",
            repository_id: "repository_0001",
            story_spec_ids: ["story_spec_0001"],
            design_spec_ids: ["design_spec_0001"],
            title: "实现提示组件",
            plan_status: "draft",
            execution_status: "planning",
          },
        ],
        workspace_sessions: [],
      });
    }
    return jsonResponse({});
  });
}

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}
