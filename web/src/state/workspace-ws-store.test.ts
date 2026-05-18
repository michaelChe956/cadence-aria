import { beforeEach, describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";

describe("workspace ws store", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("clears partial streaming content when an active run is aborted", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("prepare_context");

    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("keeps streaming content while the stage remains running", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("running");

    expect(useWorkspaceStore.getState().streamingContent).toBe("partial output");
  });

  it("tracks and resolves pending permission requests", () => {
    const store = useWorkspaceStore.getState();
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(1);

    store.resolvePermissionRequest("perm_001");

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
  });

  it("deduplicates pending permission requests by id", () => {
    const store = useWorkspaceStore.getState();

    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo clippy",
      risk_level: "high",
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toEqual([
      {
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo clippy",
        risk_level: "high",
      },
    ]);
  });

  it("updates provider status independently from workspace stage", () => {
    const store = useWorkspaceStore.getState();

    store.setProviderStatus("waiting_approval");

    expect(useWorkspaceStore.getState().providerStatus).toBe("waiting_approval");
    expect(useWorkspaceStore.getState().stage).toBe("prepare_context");
  });

  it("upserts execution events by id so command completion replaces running state", () => {
    const store = useWorkspaceStore.getState();

    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "started",
      title: "Command started",
      detail: null,
      command: "pwd",
      cwd: "/tmp/repo",
      output: null,
      exit_code: null,
    });
    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "completed",
      title: "Command completed",
      detail: "exit code 0",
      command: "pwd",
      cwd: "/tmp/repo",
      output: "/tmp/repo\n",
      exit_code: 0,
    });

    expect(useWorkspaceStore.getState().executionEvents).toEqual([
      {
        event_id: "command_cmd_001",
        kind: "command",
        status: "completed",
        title: "Command completed",
        detail: "exit code 0",
        command: "pwd",
        cwd: "/tmp/repo",
        output: "/tmp/repo\n",
        exit_code: 0,
      },
    ]);
  });

  it("clears permission state when a session snapshot is applied", () => {
    const store = useWorkspaceStore.getState();
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });
    store.setProviderStatus("waiting_approval");
    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "started",
      title: "Command started",
      detail: null,
      command: "pwd",
      cwd: "/tmp/repo",
      output: null,
      exit_code: null,
    });

    store.setSessionState({
      session_id: "session_002",
      workspace_type: "documentation",
      stage: "prepare_context",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "fake", reviewer: null },
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
    expect(useWorkspaceStore.getState().providerStatus).toBe("starting");
    expect(useWorkspaceStore.getState().executionEvents).toHaveLength(0);
  });
});
