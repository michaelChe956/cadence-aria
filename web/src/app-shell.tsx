import { useEffect, useRef, useState } from "react";
import {
  advanceTask,
  confirmTask,
  createTask,
  getProjection,
  rollbackPreview,
  rollbackTask,
  stopTask,
} from "./api/client";
import type {
  CreateTaskRequest,
  RollbackPreviewResponse,
  TaskListResponse,
  WebEvent,
} from "./api/types";
import { ActionComposer } from "./components/action/ActionComposer";
import { AutoActionStatus } from "./components/action/AutoActionStatus";
import { DiagnosticsPanel } from "./components/diagnostics/DiagnosticsPanel";
import { EvidencePanel } from "./components/evidence/EvidencePanel";
import { FlowRail } from "./components/flow/FlowRail";
import { NodeWorkspace } from "./components/node/NodeWorkspace";
import { TaskSwitcher } from "./components/shell/TaskSwitcher";
import { TopStatusBar } from "./components/shell/TopStatusBar";
import { NewTaskPanel } from "./components/task/NewTaskPanel";
import { RollbackDialog } from "./components/rollback/RollbackDialog";
import { createWorkbenchStore } from "./state/workbench-store";
import type { WorkbenchTab } from "./state/workbench-store";

export function AppShell() {
  const [store] = useState(() => createWorkbenchStore());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tasks, setTasks] = useState<TaskListResponse["tasks"]>([]);
  const [rollbackOpen, setRollbackOpen] = useState(false);
  const [rollbackPreviewState, setRollbackPreviewState] =
    useState<RollbackPreviewResponse | null>(null);
  const [lastCheckpointId, setLastCheckpointId] = useState<string | null>(null);
  const [sseConnected, setSseConnected] = useState(false);
  const [, setProjectionVersion] = useState(0);
  const projection = store.snapshot.projection;
  const activeTaskId = projection?.active_task_id ?? null;

  useEffect(() => {
    if (!activeTaskId || typeof EventSource === "undefined") {
      return;
    }
    const eventSource = new EventSource("/api/events");
    const refreshEventTypes = new Set([
      "projection_updated",
      "node_completed",
      "artifact_written",
      "rollback_completed",
    ]);
    const eventTypes = [
      "projection_updated",
      "node_started",
      "node_completed",
      "node_failed",
      "paused_for_approval",
      "provider_output",
      "artifact_written",
      "gate_blocked",
      "checkpoint_created",
      "rollback_previewed",
      "rollback_completed",
      "error",
    ];
    const handleEvent = (message: MessageEvent) => {
      const event = parseWebEvent(message.data);
      if (!event) {
        return;
      }
      store.pushEvent(event);
      setProjectionVersion((version) => version + 1);
      if (refreshEventTypes.has(event.event_type)) {
        const taskId = event.task_id ?? activeTaskId;
        void getProjection(taskId, store.snapshot.selectedNodeId ?? undefined)
          .then((nextProjection) => {
            store.setProjection(nextProjection);
            setProjectionVersion((version) => version + 1);
          })
          .catch((reason) => {
            setError(reason instanceof Error ? reason.message : "projection refresh failed");
          });
      }
    };
    eventSource.onopen = () => setSseConnected(true);
    eventSource.onerror = () => setSseConnected(false);
    eventTypes.forEach((eventType) => eventSource.addEventListener(eventType, handleEvent));
    return () => {
      eventTypes.forEach((eventType) => eventSource.removeEventListener(eventType, handleEvent));
      eventSource.close();
      setSseConnected(false);
    };
  }, [activeTaskId, store]);

  async function handleCreateTask(payload: CreateTaskRequest) {
    setBusy(true);
    setError(null);
    try {
      const created = await createTask(payload);
      const projection = await getProjection(created.task_id);
      store.setProjection(projection);
      setLastCheckpointId(projection.pending_provider_step?.checkpoint_id ?? null);
      setTasks((current) => [
        ...current.filter((task) => task.task_id !== created.task_id),
        { task_id: created.task_id, change_id: created.change_id, phase: created.phase },
      ]);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create task failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleSelectTask(taskId: string) {
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const projection = await getProjection(taskId);
      store.setProjection(projection);
      setLastCheckpointId(projection.pending_provider_step?.checkpoint_id ?? null);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "load task failed");
    } finally {
      setBusy(false);
    }
  }

  function handleSelectNode(nodeId: string) {
    store.selectNode(nodeId);
    setProjectionVersion((version) => version + 1);
  }

  function handleSelectTab(tab: WorkbenchTab) {
    store.selectTab(tab);
    setProjectionVersion((version) => version + 1);
  }

  async function handleConfirmProvider(payload: {
    checkpoint_id: string;
    prompt: string;
    policy_override?: string | null;
  }) {
    const taskId = projection?.active_task_id;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const confirmed = await confirmTask(taskId, payload);
      store.setProjection(await getProjection(taskId, confirmed.node_id));
      store.selectTab("run");
      setLastCheckpointId(payload.checkpoint_id);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "confirm task failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleStopTask() {
    const taskId = projection?.active_task_id;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await stopTask(taskId);
      store.setProjection(await getProjection(taskId, store.snapshot.selectedNodeId ?? undefined));
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "stop task failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleAdvanceTask() {
    const taskId = projection?.active_task_id;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await advanceTask(taskId);
      const nextProjection = await getProjection(taskId, store.snapshot.selectedNodeId ?? undefined);
      store.setProjection(nextProjection);
      setLastCheckpointId(nextProjection.pending_provider_step?.checkpoint_id ?? lastCheckpointId);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "advance task failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleRollbackPreview(checkpointId: string) {
    const taskId = projection?.active_task_id;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setRollbackPreviewState(await rollbackPreview(taskId, checkpointId));
      setRollbackOpen(true);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "rollback preview failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleRollbackConfirm(payload: {
    checkpoint_id: string;
    force_when_dirty: boolean;
  }) {
    const taskId = projection?.active_task_id;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await rollbackTask(taskId, payload);
      store.setProjection(await getProjection(taskId, store.snapshot.selectedNodeId ?? undefined));
      setRollbackOpen(false);
      setRollbackPreviewState(null);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "rollback failed");
    } finally {
      setBusy(false);
    }
  }

  const selectedNodeContext = projection?.selected_node_context ?? {
    node_id: store.snapshot.selectedNodeId,
    overview: {},
    inputs: [],
    run: [],
    outputs: [],
    diffs: [],
  };

  return (
    <div className="min-h-screen bg-[#05070d] text-slate-100">
      <header
        role="banner"
        className="flex h-12 items-center justify-between border-b border-cyan-400/10 bg-[#070d14] px-4"
      >
        <strong className="tracking-wide text-cyan-100">Aria Web</strong>
        <TaskSwitcher
          tasks={tasks}
          activeTaskId={projection?.active_task_id ?? null}
          onSelectTask={handleSelectTask}
        />
      </header>
      <TopStatusBar
        projection={
          projection
            ? { ...projection, sse_connected: sseConnected, running_state: busy ? "running" : "idle" }
            : null
        }
      />
      {error ? (
        <div role="alert" className="border-b border-danger/30 bg-red-50 px-4 py-2 text-sm text-danger">
          {error}
        </div>
      ) : null}
      <FlowRail
        timeline={projection?.timeline ?? []}
        selectedNodeId={store.snapshot.selectedNodeId}
        onSelectNode={handleSelectNode}
      />
      <main className="grid min-h-[calc(100vh-20rem)] grid-cols-1 gap-4 p-4 lg:grid-cols-[minmax(0,1fr)_24rem]">
        <section className="min-w-0 space-y-4">
          <NewTaskPanel onCreateTask={handleCreateTask} busy={busy} />
          {projection?.active_task_id && !projection.pending_provider_step ? (
            <section className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
              <div className="flex gap-2">
                <button
                  type="button"
                  className="rounded-md bg-cyan-300 px-3 py-2 text-sm font-semibold text-slate-950 shadow-lg shadow-cyan-500/20 disabled:opacity-50"
                  disabled={busy}
                  onClick={handleAdvanceTask}
                >
                  推进
                </button>
                {lastCheckpointId ? (
                  <button
                    type="button"
                    className="rounded-md border border-white/10 px-3 py-2 text-sm text-slate-200 hover:border-cyan-300/50"
                    onClick={() => void handleRollbackPreview(lastCheckpointId)}
                  >
                    回退
                  </button>
                ) : null}
              </div>
            </section>
          ) : null}
          {projection?.pending_provider_step ? (
            <ActionComposer
              pendingStep={projection.pending_provider_step}
              onConfirm={handleConfirmProvider}
              onRollback={handleRollbackPreview}
              onStop={handleStopTask}
              running={busy}
            />
          ) : busy ? (
            <AutoActionStatus
              currentAction="运行中"
              events={store.snapshot.events}
              onStop={handleStopTask}
            />
          ) : (
            <ActionComposer
              pendingStep={null}
              onConfirm={handleConfirmProvider}
              onRollback={handleRollbackPreview}
              onStop={handleStopTask}
              running={false}
            />
          )}
          <ProviderStreamPanel events={store.snapshot.events} run={selectedNodeContext.run} />
          <div className="rounded-xl border border-white/10 bg-white/[0.03] p-4">
            <NodeWorkspace
              context={selectedNodeContext}
              selectedTab={store.snapshot.selectedTab}
              onSelectTab={handleSelectTab}
            />
          </div>
        </section>
        <EvidencePanel artifacts={projection?.artifact_index ?? []} diagnostics={projection?.diagnostics ?? []} />
      </main>
      <RollbackDialog
        open={rollbackOpen}
        preview={rollbackPreviewState}
        onConfirm={handleRollbackConfirm}
        onOpenChange={setRollbackOpen}
      />
      <DiagnosticsPanel diagnostics={projection?.diagnostics ?? []} />
    </div>
  );
}

function ProviderStreamPanel({ events, run }: { events: WebEvent[]; run: unknown[] }) {
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const messages = [
    ...run.map((entry, index) => streamMessageFromRunEntry(entry, `run-${index}`)),
    ...events.map((event) => streamMessageFromEvent(event)),
  ].filter((message): message is ProviderStreamMessage => Boolean(message));

  useEffect(() => {
    bottomRef.current?.scrollIntoView?.({ block: "end" });
  }, [messages.length]);

  return (
    <section
      role="region"
      aria-label="Provider stream"
      className="rounded-xl border border-cyan-300/15 bg-black/45 p-4 shadow-[0_0_45px_rgba(34,211,238,0.08)]"
    >
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-lg font-semibold text-slate-100">Provider stream</h2>
        <span className="rounded-full border border-emerald-300/20 bg-emerald-300/10 px-3 py-1 text-xs text-emerald-100">
          live event log
        </span>
      </div>
      <div className="min-h-40 max-h-72 overflow-auto rounded-lg border border-white/10 bg-[#030712] p-3">
        {messages.length > 0 ? (
          <ul aria-label="Provider output messages" className="space-y-2">
            {messages.map((message) => (
              <li
                key={message.id}
                aria-label={`${message.nodeId} ${message.stream}`}
                className={
                  message.kind === "provider"
                    ? "max-w-[92%] rounded-md border border-cyan-300/15 bg-cyan-300/10 px-3 py-2 text-xs text-cyan-50"
                    : "max-w-[92%] rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-xs text-slate-300"
                }
              >
                <div className="mb-1 flex items-center gap-2 font-mono text-[11px] text-slate-400">
                  {message.kind === "provider" ? (
                    <span className="rounded bg-cyan-300/10 px-1 py-0.5 text-cyan-200">
                      provider_output
                    </span>
                  ) : null}
                  <span>{message.nodeId}</span>
                  <span>{message.stream}</span>
                </div>
                <pre className="whitespace-pre-wrap break-words font-mono leading-5">
                  {message.text}
                </pre>
              </li>
            ))}
          </ul>
        ) : (
          <div className="font-mono text-xs leading-5 text-cyan-100">等待 provider 输出...</div>
        )}
        <div ref={bottomRef} />
      </div>
    </section>
  );
}

type ProviderStreamMessage = {
  id: string;
  kind: "provider" | "event";
  nodeId: string;
  stream: string;
  text: string;
};

function streamMessageFromRunEntry(entry: unknown, id: string): ProviderStreamMessage | null {
  const record = asRecord(entry);
  if (!record) {
    return null;
  }
  const stream = text(record.stream, "stdout");
  const textValue = text(record.text ?? record.message ?? JSON.stringify(record));
  return {
    id,
    kind: "provider",
    nodeId: text(record.node_id, "run"),
    stream,
    text: textValue,
  };
}

function streamMessageFromEvent(event: WebEvent): ProviderStreamMessage | null {
  const payload = asRecord(event.payload);
  if (event.event_type === "provider_output" && payload) {
    const textValue = text(payload.text);
    if (!textValue) {
      return null;
    }
    return {
      id: `event-${event.cursor}`,
      kind: "provider",
      nodeId: text(payload.node_id, "node"),
      stream: text(payload.stream, "stdout"),
      text: textValue,
    };
  }
  return {
    id: `event-${event.cursor}`,
    kind: "event",
    nodeId: "event",
    stream: String(event.cursor),
    text: event.event_type,
  };
}

function parseWebEvent(data: string): WebEvent | null {
  try {
    const parsed = JSON.parse(data) as WebEvent;
    if (typeof parsed.event_type !== "string" || typeof parsed.cursor !== "number") {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
}

function text(value: unknown, fallback = "") {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}
