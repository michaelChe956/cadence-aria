import { useState } from "react";
import { createTask, getProjection } from "./api/client";
import type { CreateTaskRequest, TaskListResponse } from "./api/types";
import { DiagnosticsPanel } from "./components/diagnostics/DiagnosticsPanel";
import { EvidencePanel } from "./components/evidence/EvidencePanel";
import { FlowRail } from "./components/flow/FlowRail";
import { NodeWorkspace } from "./components/node/NodeWorkspace";
import { TaskSwitcher } from "./components/shell/TaskSwitcher";
import { TopStatusBar } from "./components/shell/TopStatusBar";
import { NewTaskPanel } from "./components/task/NewTaskPanel";
import { createWorkbenchStore } from "./state/workbench-store";
import type { WorkbenchTab } from "./state/workbench-store";

export function AppShell() {
  const [store] = useState(() => createWorkbenchStore());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tasks, setTasks] = useState<TaskListResponse["tasks"]>([]);
  const [, setProjectionVersion] = useState(0);
  const projection = store.snapshot.projection;

  async function handleCreateTask(payload: CreateTaskRequest) {
    setBusy(true);
    setError(null);
    try {
      const created = await createTask(payload);
      const projection = await getProjection(created.task_id);
      store.setProjection(projection);
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
      store.setProjection(await getProjection(taskId));
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

  const selectedNodeContext = projection?.selected_node_context ?? {
    node_id: store.snapshot.selectedNodeId,
    overview: {},
    inputs: [],
    run: [],
    outputs: [],
    diffs: [],
  };

  return (
    <div className="min-h-screen bg-[#eef3f4] text-ink">
      <header
        role="banner"
        className="flex h-12 items-center justify-between border-b border-line bg-white px-4"
      >
        <strong>Aria Web</strong>
        <TaskSwitcher
          tasks={tasks}
          activeTaskId={projection?.active_task_id ?? null}
          onSelectTask={handleSelectTask}
        />
      </header>
      <NewTaskPanel onCreateTask={handleCreateTask} busy={busy} />
      <TopStatusBar
        projection={
          projection
            ? { ...projection, sse_connected: true, running_state: busy ? "running" : "idle" }
            : null
        }
      />
      {error ? (
        <div role="alert" className="border-b border-danger/30 bg-red-50 px-4 py-2 text-sm text-danger">
          {error}
        </div>
      ) : null}
      <div className="grid min-h-[calc(100vh-12rem)] grid-cols-[18rem_minmax(0,1fr)_24rem]">
        <FlowRail
          timeline={projection?.timeline ?? []}
          selectedNodeId={store.snapshot.selectedNodeId}
          onSelectNode={handleSelectNode}
        />
        <NodeWorkspace
          context={selectedNodeContext}
          selectedTab={store.snapshot.selectedTab}
          onSelectTab={handleSelectTab}
        />
        <EvidencePanel artifacts={projection?.artifact_index ?? []} diagnostics={projection?.diagnostics ?? []} />
      </div>
      <DiagnosticsPanel diagnostics={projection?.diagnostics ?? []} />
    </div>
  );
}
