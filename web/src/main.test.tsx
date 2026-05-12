import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppShell } from "./main";

describe("AppShell", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders the first-screen workbench shell", () => {
    render(<AppShell />);
    expect(screen.getByRole("banner")).toHaveTextContent("Aria Web");
    expect(screen.getByRole("navigation", { name: "Node flow" })).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveTextContent("Node Workspace");
  });

  it("creates a task then advances into provider confirmation", async () => {
    let advanced = false;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/tasks") {
          return jsonResponse({
            task_id: "task_0001",
            session_id: "sess_task_0001",
            change_id: "aria-fibonacci-square",
            phase: "intake",
          });
        }
        if (url === "/api/tasks/task_0001/advance") {
          advanced = true;
          return jsonResponse({ status: "paused_for_approval", pending_step: pendingStep() });
        }
        if (url.startsWith("/api/projection")) {
          return jsonResponse(projection(advanced ? pendingStep() : null));
        }
        return jsonResponse({});
      }),
    );

    render(<AppShell />);
    await userEvent.type(screen.getByLabelText("任务请求"), "实现 Fibonacci square sum");
    await userEvent.type(screen.getByLabelText("change id"), "aria-fibonacci-square");
    await userEvent.click(screen.getByRole("button", { name: "新建任务" }));
    await screen.findByRole("button", { name: "推进" });
    await userEvent.click(screen.getByRole("button", { name: "推进" }));

    expect(await screen.findByLabelText("Provider prompt")).toBeInTheDocument();
  });

  it("loads the confirmed provider node context after confirmation", async () => {
    let advanced = false;
    let confirmed = false;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/tasks") {
          return jsonResponse({
            task_id: "task_0001",
            session_id: "sess_task_0001",
            change_id: "aria-fibonacci-square",
            phase: "intake",
          });
        }
        if (url === "/api/tasks/task_0001/advance") {
          advanced = true;
          return jsonResponse({ status: "paused_for_approval", pending_step: pendingStep() });
        }
        if (url === "/api/tasks/task_0001/confirm") {
          confirmed = true;
          return jsonResponse({ status: "provider_started", node_id: "N16", turn_id: "turn_0001" });
        }
        if (url === "/api/projection?task_id=task_0001&node_id=N16") {
          return jsonResponse(projectionWithRunOutput());
        }
        if (url.startsWith("/api/projection")) {
          return jsonResponse(projection(advanced && !confirmed ? pendingStep() : null));
        }
        return jsonResponse({});
      }),
    );

    render(<AppShell />);
    await userEvent.type(screen.getByLabelText("任务请求"), "实现 Fibonacci square sum");
    await userEvent.type(screen.getByLabelText("change id"), "aria-fibonacci-square");
    await userEvent.click(screen.getByRole("button", { name: "新建任务" }));
    await userEvent.click(await screen.findByRole("button", { name: "推进" }));
    await userEvent.click(await screen.findByRole("button", { name: "确认执行" }));

    expect(await screen.findByText(/provider_output/)).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveTextContent("N16");
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
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/tasks") {
          return jsonResponse({
            task_id: "task_0001",
            session_id: "sess_task_0001",
            change_id: "aria-fibonacci-square",
            phase: "intake",
          });
        }
        if (url === "/api/projection?task_id=task_0001") {
          if (refreshed) {
            return jsonResponse(projectionWithRunOutput());
          }
          return jsonResponse(projection(null));
        }
        if (url.startsWith("/api/projection")) {
          return jsonResponse(projection(null));
        }
        return jsonResponse({});
      }),
    );

    render(<AppShell />);
    await userEvent.type(screen.getByLabelText("任务请求"), "实现 Fibonacci square sum");
    await userEvent.type(screen.getByLabelText("change id"), "aria-fibonacci-square");
    await userEvent.click(screen.getByRole("button", { name: "新建任务" }));
    expect(eventSources).toHaveLength(1);

    refreshed = true;
    eventSources[0].emit("projection_updated", {
      cursor: 1,
      event_type: "projection_updated",
      task_id: "task_0001",
      payload: {},
    });

    expect(await screen.findByText(/provider_output/)).toBeInTheDocument();
  });
});

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
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
