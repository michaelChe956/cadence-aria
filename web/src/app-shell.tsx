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
import { ProjectManagementWorkbench } from "./components/project/ProjectManagementWorkbench";
import { ExecutionSummaryStrip } from "./components/shell/ExecutionSummaryStrip";
import { TaskSwitcher } from "./components/shell/TaskSwitcher";
import { TopStatusBar } from "./components/shell/TopStatusBar";
import { WorkbenchSurface } from "./components/shell/WorkbenchSurface";
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
      if (event.task_id && event.task_id !== activeTaskId) {
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

  const executionHeader = (
    <div className="flex min-h-10 flex-wrap items-center justify-between gap-3">
      <div className="min-w-0">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <strong className="text-base font-semibold text-[var(--aria-ink)]">Aria Web</strong>
          <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
            Issue {executionContext.issueId}
          </span>
          <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
            Workspace {executionContext.workspaceId}
          </span>
        </div>
      </div>
      <div className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-2">
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
          className="inline-flex h-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          返回项目管理
        </button>
      </div>
    </div>
  );

  return (
    <>
      <WorkbenchSurface
        header={executionHeader}
        statusBar={
          <TopStatusBar
            projection={
              projection
                ? {
                    ...projection,
                    sse_connected: sseConnected,
                    running_state: busy ? "running" : "idle",
                  }
                : null
            }
          />
        }
        alert={error}
        mainLabel="Aria workbench"
        main={
          <section role="region" aria-label="Interaction window" className="min-w-0 space-y-4">
            <ExecutionSummaryStrip
              activeTaskId={projection?.active_task_id ?? executionContext.taskId}
              selectedNodeId={store.snapshot.selectedNodeId}
              nodeCount={projection?.timeline.length ?? 0}
              artifactCount={projection?.artifact_index.length ?? 0}
              eventCount={store.snapshot.events.length}
            />
            <NewTaskPanel onCreateTask={handleCreateTask} busy={busy} />
            {projection?.active_task_id && !projection.pending_provider_step ? (
              <section className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-3">
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
                    disabled={busy}
                    onClick={handleAdvanceTask}
                  >
                    推进
                  </button>
                  {lastCheckpointId ? (
                    <button
                      type="button"
                      className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-4 text-sm font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
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
        }
        aside={
          <>
            <FlowRail
              timeline={projection?.timeline ?? []}
              selectedNodeId={store.snapshot.selectedNodeId}
              onSelectNode={(nodeId) => void handleSelectNode(nodeId)}
            />
            <NodeWorkspace
              context={selectedNodeContext}
              selectedTab={store.snapshot.selectedTab}
              onSelectTab={handleSelectTab}
            />
            <EvidencePanel
              artifacts={projection?.artifact_index ?? []}
              diagnostics={projection?.diagnostics ?? []}
            />
          </>
        }
      />
      <RollbackDialog
        open={rollbackOpen}
        preview={rollbackPreviewState}
        onConfirm={handleRollbackConfirm}
        onOpenChange={setRollbackOpen}
      />
      <DiagnosticsPanel diagnostics={projection?.diagnostics ?? []} />
    </>
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
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 text-[var(--aria-ink)]"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Interaction stream</h2>
          <p className="mt-1 text-xs font-medium text-[var(--aria-ink-muted)]">
            当前步骤：观察输出并确认下一步。
          </p>
        </div>
        <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          live event log
        </span>
      </div>
      <div
        ref={scrollBoxRef}
        className="min-h-[22rem] max-h-[34rem] overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
      >
        {messages.length > 0 ? (
          <ul aria-label="Provider output messages" aria-live="polite" className="space-y-2">
            {messages.map((message) => (
              <li
                key={message.id}
                aria-label={`${message.nodeId} ${message.stream}`}
                className={
                  message.kind === "provider"
                    ? "max-w-[92%] rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-3 py-2 text-xs text-[var(--aria-ink)]"
                    : "max-w-[92%] rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-xs text-[var(--aria-ink)]"
                }
              >
                <div className="mb-1 flex items-center gap-2 font-mono text-[11px] text-[var(--aria-ink-muted)]">
                  {message.kind === "provider" ? (
                    <span className="rounded bg-[var(--aria-primary)] px-1.5 py-0.5 font-semibold text-white">
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
          <div className="font-mono text-xs font-medium leading-5 text-[var(--aria-ink-muted)]">
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
