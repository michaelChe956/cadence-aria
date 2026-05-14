import { describe, expect, it } from "vitest";
import { createWorkbenchStore } from "./workbench-store";

describe("workbench store", () => {
  it("tracks projection, selected node, tab and event log", () => {
    const store = createWorkbenchStore();
    store.setProjection({
      workspace_root: "/tmp/workspace",
      active_task_id: "task_0001",
      active_session_id: "sess_task_0001",
      overview: { phase: "execution" },
      sessions: [],
      timeline: [{ node_id: "N16", status: "completed" }],
      artifact_index: [],
      diagnostics: [],
      available_actions: ["confirm_provider_step"],
      pending_provider_step: null,
      selected_node_context: { node_id: "N16", overview: {}, inputs: [], run: [], outputs: [], diffs: [] },
      git_summary: {
        workspace_path: "/tmp/workspace",
        branch: "main",
        head: "abc1234",
        dirty: false,
        dirty_files: [],
      },
      event_cursor: 3,
    });
    store.selectNode("N17");
    store.selectTab("outputs");
    store.restoreSelectionFromSearch(
      new URLSearchParams("node=N18&tab=run&artifact=art_0001&turn=turn_0001"),
    );
    store.pushEvent({
      cursor: 4,
      event_type: "projection_updated",
      task_id: "task_0001",
      payload: {},
    });

    expect(store.snapshot.selectedNodeId).toBe("N18");
    expect(store.snapshot.selectedTab).toBe("run");
    expect(store.snapshot.selectedArtifactRef).toBe("art_0001");
    expect(store.snapshot.selectedTurnId).toBe("turn_0001");
    expect(store.toSearchParams().toString()).toContain("node=N18");
    expect(store.snapshot.events).toHaveLength(1);
  });
});
