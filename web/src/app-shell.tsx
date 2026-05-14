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
import { LearningLabHero } from "./components/learning/LearningLabHero";
import { NodeWorkspace } from "./components/node/NodeWorkspace";
import { ProjectManagementWorkbench } from "./components/project/ProjectManagementWorkbench";
import { TaskSwitcher } from "./components/shell/TaskSwitcher";
import { TopStatusBar } from "./components/shell/TopStatusBar";
import { NewTaskPanel } from "./components/task/NewTaskPanel";
import type { ExecutionContext } from "./components/task/TaskManagementWorkbench";
import { RollbackDialog } from "./components/rollback/RollbackDialog";
import { createWorkbenchStore } from "./state/workbench-store";
import type { WorkbenchTab } from "./state/workbench-store";

export function AppShell() {
  const [store] = useState(() => createWorkbenchStore());
  const [executionContext, setExecutionContext] = useState<ExecutionContext | null>(null);
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
  const workspaceId = executionContext?.workspaceId;
  const activeTaskId = executionContext?.taskId ?? projection?.active_task_id ?? null;

  useEffect(() => {
    const previousScrollRestoration = window.history.scrollRestoration;
    window.history.scrollRestoration = "manual";
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });

    return () => {
      window.history.scrollRestoration = previousScrollRestoration;
    };
  }, []);

  useEffect(() => {
    if (!executionContext) {
      return;
    }
    let cancelled = false;
    setBusy(true);
    setError(null);
    void getProjection(executionContext.taskId, undefined, executionContext.workspaceId)
      .then((nextProjection) => {
        if (cancelled) {
          return;
        }
        store.setProjection(nextProjection);
        setLastCheckpointId(nextProjection.pending_provider_step?.checkpoint_id ?? null);
        setTasks((current) => [
          ...current.filter((task) => task.task_id !== executionContext.taskId),
          {
            task_id: executionContext.taskId,
            change_id: nextProjection.overview.change_id as string | null,
            phase: nextProjection.overview.status as string | null,
          },
        ]);
        setProjectionVersion((version) => version + 1);
      })
      .catch((reason) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : "load execution workbench failed");
        }
      })
      .finally(() => {
        if (!cancelled) {
          setBusy(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [executionContext, store]);

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
        void getProjection(taskId, store.snapshot.selectedNodeId ?? undefined, workspaceId)
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
  }, [activeTaskId, store, workspaceId]);

  async function handleCreateTask(payload: CreateTaskRequest) {
    setBusy(true);
    setError(null);
    try {
      const created = await createTask(payload);
      const projection = await getProjection(created.task_id, undefined, workspaceId);
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
      const projection = await getProjection(taskId, undefined, workspaceId);
      store.setProjection(projection);
      setLastCheckpointId(projection.pending_provider_step?.checkpoint_id ?? null);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "load task failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleSelectNode(nodeId: string) {
    store.selectNode(nodeId);
    setProjectionVersion((version) => version + 1);
    if (!activeTaskId) {
      return;
    }
    setError(null);
    try {
      const nextProjection = await getProjection(activeTaskId, nodeId, workspaceId);
      store.setProjection(nextProjection);
      setLastCheckpointId(nextProjection.pending_provider_step?.checkpoint_id ?? lastCheckpointId);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "load node context failed");
    }
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
    const taskId = projection?.active_task_id ?? executionContext?.taskId;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const confirmed = await confirmTask(taskId, payload, workspaceId);
      store.setProjection(await getProjection(taskId, confirmed.node_id, workspaceId));
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
    const taskId = projection?.active_task_id ?? executionContext?.taskId;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await stopTask(taskId, workspaceId);
      store.setProjection(
        await getProjection(taskId, store.snapshot.selectedNodeId ?? undefined, workspaceId),
      );
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "stop task failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleAdvanceTask() {
    const taskId = projection?.active_task_id ?? executionContext?.taskId;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await advanceTask(taskId, workspaceId);
      const nextProjection = await getProjection(
        taskId,
        store.snapshot.selectedNodeId ?? undefined,
        workspaceId,
      );
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
    const taskId = projection?.active_task_id ?? executionContext?.taskId;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setRollbackPreviewState(await rollbackPreview(taskId, checkpointId, workspaceId));
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
    const taskId = projection?.active_task_id ?? executionContext?.taskId;
    if (!taskId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await rollbackTask(taskId, payload, workspaceId);
      store.setProjection(
        await getProjection(taskId, store.snapshot.selectedNodeId ?? undefined, workspaceId),
      );
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

  if (!executionContext) {
    return <ProjectManagementWorkbench onOpenExecution={setExecutionContext} />;
  }

  return (
    <div className="min-h-screen text-[#241B2F]">
      <header
        role="banner"
        className="sticky top-0 z-20 flex min-h-16 flex-wrap items-center justify-between gap-3 border-b-2 border-rose-200 bg-white/88 px-4 py-3 shadow-[0_10px_30px_rgba(249,115,22,0.12)] backdrop-blur md:px-6 lg:px-8"
      >
        <div>
          <strong className="text-lg text-[#241B2F]">Aria Web</strong>
          <span className="ml-3 hidden text-sm font-semibold text-[#5E516B] sm:inline">
            playful coding workbench
          </span>
        </div>
        <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
          <span className="rounded-lg border-2 border-indigo-200 bg-indigo-50 px-3 py-1 font-mono text-xs font-bold text-indigo-950">
            {executionContext.issueId}
          </span>
          <span className="rounded-lg border-2 border-cyan-200 bg-cyan-50 px-3 py-1 font-mono text-xs font-bold text-cyan-950">
            {executionContext.workspaceId}
          </span>
          <TaskSwitcher tasks={tasks} activeTaskId={activeTaskId} onSelectTask={handleSelectTask} />
          <button
            type="button"
            onClick={() => {
              setExecutionContext(null);
              setError(null);
              setRollbackOpen(false);
              setRollbackPreviewState(null);
              setSseConnected(false);
            }}
            className="rounded-lg border-2 border-slate-300 bg-white px-3 py-1.5 text-sm font-bold text-slate-800 shadow-[0_4px_0_rgba(15,23,42,0.10)] transition-colors hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200"
          >
            返回项目管理
          </button>
        </div>
      </header>
      <TopStatusBar
        projection={
          projection
            ? { ...projection, sse_connected: sseConnected, running_state: busy ? "running" : "idle" }
            : null
        }
      />
      {error ? (
        <div
          role="alert"
          className="border-b-2 border-rose-200 bg-rose-100 px-4 py-2 text-sm font-semibold text-rose-800 md:px-6 lg:px-8"
        >
          {error}
        </div>
      ) : null}
      <main
        aria-label="Aria workbench"
        className="grid min-h-[calc(100vh-10rem)] grid-cols-1 gap-5 px-4 py-5 text-[#241B2F] md:px-6 lg:grid-cols-[minmax(0,1fr)_25rem] lg:px-8 xl:grid-cols-[minmax(0,1fr)_28rem]"
      >
        <section
          role="region"
          aria-label="Interaction window"
          className="min-w-0 space-y-5 rounded-lg border-2 border-rose-200 bg-white/82 p-4 shadow-[0_12px_0_rgba(249,115,22,0.08),0_24px_50px_rgba(190,24,93,0.12)] md:p-5"
        >
          <LearningLabHero
            activeTaskId={projection?.active_task_id ?? null}
            nodeCount={projection?.timeline.length ?? 0}
            artifactCount={projection?.artifact_index.length ?? 0}
            eventCount={store.snapshot.events.length}
            selectedNodeId={store.snapshot.selectedNodeId}
          />
          <NewTaskPanel onCreateTask={handleCreateTask} busy={busy} />
          {projection?.active_task_id && !projection.pending_provider_step ? (
            <section className="rounded-lg border-2 border-orange-200 bg-orange-50 px-4 py-3 shadow-[0_8px_0_rgba(249,115,22,0.18)]">
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  className="rounded-lg border-2 border-orange-600 bg-orange-500 px-4 py-2 text-sm font-bold text-white shadow-[0_5px_0_rgba(154,52,18,0.45)] transition-colors hover:bg-orange-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
                  disabled={busy}
                  onClick={handleAdvanceTask}
                >
                  推进
                </button>
                {lastCheckpointId ? (
                  <button
                    type="button"
                    className="rounded-lg border-2 border-rose-300 bg-white px-4 py-2 text-sm font-bold text-[#8E2D60] shadow-[0_5px_0_rgba(190,24,93,0.16)] transition-colors hover:bg-rose-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200"
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
        </section>
        <aside className="space-y-5">
          <FlowRail
            timeline={projection?.timeline ?? []}
            selectedNodeId={store.snapshot.selectedNodeId}
            onSelectNode={(nodeId) => void handleSelectNode(nodeId)}
          />
          <div className="rounded-lg border-2 border-rose-200 bg-white/92 p-4 shadow-[0_10px_0_rgba(249,115,22,0.08),0_18px_34px_rgba(190,24,93,0.12)]">
            <NodeWorkspace
              context={selectedNodeContext}
              selectedTab={store.snapshot.selectedTab}
              onSelectTab={handleSelectTab}
            />
          </div>
          <EvidencePanel
            artifacts={projection?.artifact_index ?? []}
            diagnostics={projection?.diagnostics ?? []}
          />
        </aside>
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
  const scrollBoxRef = useRef<HTMLDivElement | null>(null);
  const messages = [
    ...run.map((entry, index) => streamMessageFromRunEntry(entry, `run-${index}`)),
    ...events.map((event) => streamMessageFromEvent(event)),
  ].filter((message): message is ProviderStreamMessage => Boolean(message));

  useEffect(() => {
    const scrollBox = scrollBoxRef.current;
    if (scrollBox) {
      scrollBox.scrollTop = scrollBox.scrollHeight;
    }
  }, [messages.length]);

  return (
    <section
      role="region"
      aria-label="Provider stream"
      className="rounded-lg border-2 border-cyan-200 bg-white p-4 text-[#241B2F] shadow-[0_10px_0_rgba(6,182,212,0.12),0_18px_38px_rgba(15,118,110,0.14)]"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-xl font-black text-[#241B2F]">Interaction stream</h2>
          <p className="mt-1 text-sm font-semibold text-[#5E516B]">
            当前步骤：观察输出并确认下一步。
          </p>
        </div>
        <span className="rounded-lg border-2 border-cyan-200 bg-cyan-100 px-3 py-1 text-xs font-bold text-cyan-900">
          live event log
        </span>
      </div>
      <div
        ref={scrollBoxRef}
        className="min-h-[22rem] max-h-[34rem] overflow-auto rounded-lg border-2 border-rose-100 bg-rose-50/80 p-3 shadow-inner shadow-rose-200/70"
      >
        {messages.length > 0 ? (
          <ul aria-label="Provider output messages" aria-live="polite" className="space-y-2">
            {messages.map((message) => (
              <li
                key={message.id}
                aria-label={`${message.nodeId} ${message.stream}`}
                className={
                  message.kind === "provider"
                    ? "max-w-[92%] rounded-lg border-2 border-cyan-300 bg-cyan-100 px-3 py-2 text-xs text-cyan-950 shadow-[0_5px_0_rgba(6,182,212,0.16)]"
                    : "max-w-[92%] rounded-lg border-2 border-rose-200 bg-white px-3 py-2 text-xs text-[#241B2F] shadow-[0_5px_0_rgba(190,24,93,0.12)]"
                }
              >
                <div className="mb-1 flex items-center gap-2 font-mono text-[11px] text-[#7A6C83]">
                  {message.kind === "provider" ? (
                    <span className="rounded-md bg-orange-400 px-2 py-0.5 font-bold text-white">
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
          <div className="font-mono text-xs font-semibold leading-5 text-[#7A6C83]">
            等待 provider 输出...
          </div>
        )}
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
