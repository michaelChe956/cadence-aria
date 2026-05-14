import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppShell } from "./main";

describe("AppShell", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders task management as the default workbench", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/workspaces") return jsonResponse({ workspaces: [] });
        if (url === "/api/issues") return jsonResponse({ issues: [] });
        return jsonResponse({});
      }),
    );

    render(<AppShell />);

    expect(await screen.findByRole("heading", { name: "任务管理" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Workspace 空间" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "新建 issue" })).toBeDisabled();
  });

  it("resets page scroll to the top when the workbench loads", () => {
    stubEmptyManagementFetch();
    const scrollTo = vi.fn();
    Object.defineProperty(window, "scrollTo", { configurable: true, value: scrollTo });
    Object.defineProperty(window.history, "scrollRestoration", {
      configurable: true,
      value: "auto",
      writable: true,
    });

    render(<AppShell />);

    expect(window.history.scrollRestoration).toBe("manual");
    expect(scrollTo).toHaveBeenCalledWith({ top: 0, left: 0, behavior: "auto" });
  });

  it("places the interaction window before the workflow navigation", async () => {
    stubExecutionFetch((url) => {
      if (url === "/api/projection?task_id=task_0001&workspace_id=workspace_0001") {
        return jsonResponse(projection(null));
      }
      return null;
    });

    await openExecutionWorkbench();

    const interactionWindow = screen.getByRole("region", { name: "Interaction window" });
    const workflowMap = screen.getByRole("navigation", { name: "Workflow map" });

    expect(
      interactionWindow.compareDocumentPosition(workflowMap) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("renders an illustrated AI coding workbench in the interaction window", async () => {
    stubExecutionFetch((url) => {
      if (url === "/api/projection?task_id=task_0001&workspace_id=workspace_0001") {
        return jsonResponse(projection(null));
      }
      return null;
    });

    await openExecutionWorkbench();

    const interactionWindow = screen.getByRole("region", { name: "Interaction window" });
    expect(
      within(interactionWindow).getByRole("img", { name: "AI coding workbench illustration" }),
    ).toBeInTheDocument();
    expect(
      within(interactionWindow).getByRole("group", { name: "AI coding workbench status" }),
    ).toHaveTextContent("AI Coding Workbench");
    expect(within(interactionWindow).getByTestId("workbench-visual")).toHaveAttribute(
      "data-motion",
      "ambient",
    );
  });

  it("starts an issue with a workspace and opens the execution workbench", async () => {
    stubExecutionFetch((url) => {
      if (url === "/api/projection?task_id=task_0001&workspace_id=workspace_0001") {
        return jsonResponse(projection(null));
      }
      return null;
    });

    await openExecutionWorkbench();

    expect(await screen.findByRole("main", { name: "Aria workbench" })).toBeInTheDocument();
    expect(screen.getByText("issue_0001")).toBeInTheDocument();
    expect(screen.getByText("workspace_0001")).toBeInTheDocument();
  });

  it("starts an issue then advances into provider confirmation", async () => {
    let advanced = false;
    stubExecutionFetch((url) => {
      if (url === "/api/tasks/task_0001/advance?workspace_id=workspace_0001") {
        advanced = true;
        return jsonResponse({ status: "paused_for_approval", pending_step: pendingStep() });
      }
      if (url.startsWith("/api/projection")) {
        return jsonResponse(projection(advanced ? pendingStep() : null));
      }
      return null;
    });

    await openExecutionWorkbench();
    await screen.findByRole("button", { name: "推进" });
    await userEvent.click(screen.getByRole("button", { name: "推进" }));

    expect(await screen.findByLabelText("Provider prompt")).toBeInTheDocument();
  });

  it("loads the confirmed provider node context after confirmation", async () => {
    let advanced = false;
    let confirmed = false;
    stubExecutionFetch((url) => {
      if (url === "/api/tasks/task_0001/advance?workspace_id=workspace_0001") {
        advanced = true;
        return jsonResponse({ status: "paused_for_approval", pending_step: pendingStep() });
      }
      if (url === "/api/tasks/task_0001/confirm?workspace_id=workspace_0001") {
        confirmed = true;
        return jsonResponse({ status: "provider_started", node_id: "N16", turn_id: "turn_0001" });
      }
      if (url === "/api/projection?task_id=task_0001&node_id=N16&workspace_id=workspace_0001") {
        return jsonResponse(projectionWithRunOutput());
      }
      if (url.startsWith("/api/projection")) {
        return jsonResponse(projection(advanced && !confirmed ? pendingStep() : null));
      }
      return null;
    });

    await openExecutionWorkbench();
    await userEvent.click(await screen.findByRole("button", { name: "推进" }));
    await userEvent.click(await screen.findByRole("button", { name: "确认执行" }));

    const stream = screen.getByRole("region", { name: "Provider stream" });
    expect(await within(stream).findByText(/done/)).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveTextContent("N16");
  });

  it("loads selected node context when a workflow node is clicked", async () => {
    const fetchSpy = stubExecutionFetch((url) => {
      if (url === "/api/projection?task_id=task_0001&node_id=N17&workspace_id=workspace_0001") {
        return jsonResponse(projectionWithSelectedNode("N17"));
      }
      if (url.startsWith("/api/projection")) {
        return jsonResponse(projectionWithSelectedNode("N16"));
      }
      return null;
    });

    await openExecutionWorkbench();
    await userEvent.click(await screen.findByRole("button", { name: /N17/ }));

    const nodeDetails = screen.getByRole("region", { name: "Node workspace details" });
    const summary = await within(nodeDetails).findByRole("group", { name: "当前节点摘要" });
    expect(within(summary).getByText("N17")).toBeInTheDocument();
    expect(within(summary).getByText("running")).toBeInTheDocument();
    expect(fetchSpy).toHaveBeenCalledWith(
      "/api/projection?task_id=task_0001&node_id=N17&workspace_id=workspace_0001",
      expect.objectContaining({
        headers: expect.objectContaining({ "content-type": "application/json" }),
      }),
    );
  });

  it("refreshes the projection when an SSE projection update arrives", async () => {
    const eventSources: MockEventSource[] = [];
    let refreshed = false;
    vi.stubGlobal(
      "EventSource",
      class extends MockEventSource {
        constructor(url: string) {
          super(url);
          eventSources.push(this);
        }
      },
    );
    stubExecutionFetch((url) => {
      if (url === "/api/projection?task_id=task_0001&workspace_id=workspace_0001") {
        if (refreshed) {
          return jsonResponse(projectionWithRunOutput());
        }
        return jsonResponse(projection(null));
      }
      if (url.startsWith("/api/projection")) {
        return jsonResponse(projection(null));
      }
      return null;
    });

    await openExecutionWorkbench();
    await waitFor(() => expect(eventSources).toHaveLength(1));

    refreshed = true;
    eventSources[0].emit("projection_updated", {
      cursor: 1,
      event_type: "projection_updated",
      task_id: "task_0001",
      payload: {},
    });

    const stream = screen.getByRole("region", { name: "Provider stream" });
    expect(await within(stream).findByText(/done/)).toBeInTheDocument();
  });

  it("appends provider output SSE chunks to the workspace stream", async () => {
    const eventSources: MockEventSource[] = [];
    vi.stubGlobal(
      "EventSource",
      class extends MockEventSource {
        constructor(url: string) {
          super(url);
          eventSources.push(this);
        }
      },
    );
    stubExecutionFetch((url) => {
      if (url.startsWith("/api/projection")) {
        return jsonResponse(projection(null));
      }
      return null;
    });

    await openExecutionWorkbench();
    await waitFor(() => expect(eventSources).toHaveLength(1));

    eventSources[0].emit("provider_output", {
      cursor: 2,
      event_type: "provider_output",
      task_id: "task_0001",
      payload: {
        node_id: "N16",
        provider_run_id: "run_n16_0001",
        stream: "stdout",
        text: "streamed provider line",
      },
    });

    expect(await screen.findByText(/streamed provider line/)).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Provider stream" })).toHaveTextContent("N16");
    expect(screen.getByRole("list", { name: "Provider output messages" })).toBeInTheDocument();
    expect(screen.getByRole("listitem", { name: /N16 stdout/ })).toHaveTextContent(
      "streamed provider line",
    );
  });
});

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}

function stubEmptyManagementFetch() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === "/api/workspaces") return jsonResponse({ workspaces: [] });
      if (url === "/api/issues") return jsonResponse({ issues: [] });
      return jsonResponse({});
    }),
  );
}

function stubExecutionFetch(
  handler: (url: string, init?: RequestInit) => Promise<Response> | null,
) {
  const fetchSpy = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const managementResponse = executionManagementResponse(url);
    if (managementResponse) {
      return managementResponse;
    }
    const response = handler(url, init);
    return response ?? jsonResponse({});
  });
  vi.stubGlobal("fetch", fetchSpy);
  return fetchSpy;
}

function executionManagementResponse(url: string) {
  if (url === "/api/workspaces") return jsonResponse({ workspaces: [workspaceFixture()] });
  if (url === "/api/issues") return jsonResponse({ issues: [issueFixture()] });
  if (url === "/api/issues/issue_0001/start") {
    return jsonResponse({
      issue_id: "issue_0001",
      workspace_id: "workspace_0001",
      task_id: "task_0001",
      session_id: "sess_task_0001",
      status: "started",
    });
  }
  return null;
}

async function openExecutionWorkbench() {
  render(<AppShell />);
  await userEvent.click(await screen.findByRole("button", { name: "Start" }));
  await userEvent.selectOptions(screen.getByLabelText("启动 workspace"), "workspace_0001");
  await userEvent.click(screen.getByRole("button", { name: "确认 Start" }));
  return screen.findByRole("main", { name: "Aria workbench" });
}

function workspaceFixture() {
  return {
    workspace_id: "workspace_0001",
    name: "Repo",
    path: "/tmp/repo",
    default_policy_preset: "manual-write",
    default_provider_mode: "fake",
    created_at: "2026-05-14T00:00:00Z",
    updated_at: "2026-05-14T00:00:00Z",
  };
}

function issueFixture() {
  return {
    issue_id: "issue_0001",
    title: "Implement picker",
    description: "Select repo",
    status: "draft",
    workspace_id: null,
    task_id: null,
    session_id: null,
    change_id: "implement-picker",
    created_at: "2026-05-14T00:00:00Z",
    updated_at: "2026-05-14T00:00:00Z",
  };
}

function pendingStep() {
  return {
    node_id: "N16",
    provider_type: "codex",
    runtime_role: "executor",
    adapter_role: "executor",
    prompt: "实现函数",
    input_summary: {},
    canonical_input_refs: ["worktask:work_wt_001"],
    context_files: ["openspec/changes/aria-fibonacci-square/tasks.md"],
    output_schema: "schema://aria/artifacts/coding_report/v1",
    allowed_write_scope: ["src/", "tests/"],
    forbidden_actions: ["修改 cadence/project-rules"],
    verification_commands: ["cargo test --locked -j 1"],
    checkpoint_id: "ckpt_0001",
  };
}

function projection(pending_provider_step: ReturnType<typeof pendingStep> | null) {
  return {
    workspace_root: "/tmp/workspace",
    active_task_id: "task_0001",
    active_session_id: "sess_task_0001",
    overview: { status: pending_provider_step ? "paused" : "intake" },
    sessions: [],
    timeline: [],
    artifact_index: [],
    diagnostics: [],
    available_actions: pending_provider_step ? ["confirm_provider_step"] : [],
    pending_provider_step,
    selected_node_context: { node_id: null, overview: {}, inputs: [], run: [], outputs: [], diffs: [] },
    git_summary: { workspace_path: "/tmp/workspace", branch: "main", head: "abc1234", dirty: false, dirty_files: [] },
    event_cursor: 0,
  };
}

function projectionWithRunOutput() {
  return {
    ...projection(null),
    timeline: [{ node_id: "N16", status: "completed", artifact_count: 1 }],
    artifact_index: [
      {
        artifact_ref: "coding_report_work_wt_001_0001",
        artifact_kind: "coding_report",
        path: ".aria/runtime/tasks/task_0001/artifacts/execution/0000.json",
        producer_node: "N16",
      },
    ],
    selected_node_context: {
      node_id: "N16",
      overview: { node_id: "N16", status: "completed" },
      inputs: [],
      run: [{ kind: "provider_output", stream: "stdout", text: "done" }],
      outputs: [{ artifact_ref: "coding_report_work_wt_001_0001" }],
      diffs: [],
    },
  };
}

function projectionWithSelectedNode(nodeId: string) {
  return {
    ...projection(null),
    timeline: [
      { node_id: "N16", status: "completed", provider_type: "codex", artifact_count: 1 },
      { node_id: "N17", status: "running", provider_type: "codex", attempt: 2, artifact_count: 3 },
    ],
    selected_node_context: {
      node_id: nodeId,
      overview: {
        node_id: nodeId,
        status: nodeId === "N17" ? "running" : "completed",
        provider_type: "codex",
        attempt: nodeId === "N17" ? 2 : 1,
        artifact_count: nodeId === "N17" ? 3 : 1,
      },
      inputs: [],
      run: [{ kind: "provider_output", stream: "stdout", text: `${nodeId} context loaded` }],
      outputs: [{ artifact_ref: `coding_report_${nodeId}` }],
      diffs: [],
    },
  };
}

class MockEventSource {
  readonly url: string;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private listeners = new Map<string, Array<(event: MessageEvent) => void>>();

  constructor(url: string) {
    this.url = url;
    queueMicrotask(() => this.onopen?.());
  }

  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  removeEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.set(
      type,
      (this.listeners.get(type) ?? []).filter((candidate) => candidate !== listener),
    );
  }

  close() {}

  emit(type: string, payload: unknown) {
    const event = new MessageEvent(type, { data: JSON.stringify(payload) });
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event);
    }
  }
}
