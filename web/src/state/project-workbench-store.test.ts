import { describe, expect, it } from "vitest";
import { createProjectWorkbenchStore } from "./project-workbench-store";

describe("project workbench store", () => {
  it("selects project issue and keeps recent provider events", () => {
    const store = createProjectWorkbenchStore();
    const project = {
      project_id: "project_0001",
      name: "Aria",
      description: null,
      created_at: "now",
      updated_at: "now",
      last_opened_at: null,
    };
    const issue = {
      issue_id: "issue_0001",
      project_id: "project_0001",
      repo_id: "repo_0001",
      title: "Login",
      description: null,
      change_id: "login",
      phase: "clarification" as const,
      status: "draft" as const,
      active_binding_id: null,
      created_at: "now",
      updated_at: "now",
    };

    expect(store.snapshot).toEqual({
      projects: [],
      issues: [],
      selectedProjectId: null,
      selectedIssueId: null,
      events: [],
    });

    store.setProjects([project]);
    store.selectProject("project_0001");
    store.setIssues([issue]);
    store.selectIssue("issue_0001");

    for (let cursor = 1; cursor <= 201; cursor += 1) {
      store.pushEvent({
        cursor,
        event_type: "issue.updated",
        task_id: null,
        project_id: "project_0001",
        issue_id: "issue_0001",
        binding_id: null,
        payload: {},
      });
    }
    const providerEvent = {
      cursor: 202,
      event_type: "provider.output_stream",
      task_id: null,
      project_id: "project_0001",
      issue_id: "issue_0001",
      binding_id: null,
      payload: { text: "chunk" },
    };
    store.pushEvent(providerEvent);

    expect(store.snapshot.selectedProjectId).toBe("project_0001");
    expect(store.snapshot.selectedIssueId).toBe("issue_0001");
    expect(store.snapshot.projects).toEqual([project]);
    expect(store.snapshot.issues).toEqual([issue]);
    expect(store.snapshot.events).toHaveLength(200);
    expect(store.snapshot.events[0].cursor).toBe(3);
    expect(store.snapshot.events.at(-1)).toEqual(providerEvent);
  });
});
