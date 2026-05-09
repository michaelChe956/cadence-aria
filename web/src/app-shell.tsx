import { useState } from "react";
import { createTask, getProjection } from "./api/client";
import { NewTaskPanel } from "./components/task/NewTaskPanel";
import { createWorkbenchStore } from "./state/workbench-store";
import type { CreateTaskRequest } from "./api/types";

export function AppShell() {
  const [store] = useState(() => createWorkbenchStore());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [, setProjectionVersion] = useState(0);

  async function handleCreateTask(payload: CreateTaskRequest) {
    setBusy(true);
    setError(null);
    try {
      const created = await createTask(payload);
      const projection = await getProjection(created.task_id);
      store.setProjection(projection);
      setProjectionVersion((version) => version + 1);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create task failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-screen bg-[#eef3f4] text-ink">
      <header
        role="banner"
        className="flex h-12 items-center justify-between border-b border-line bg-white px-4"
      >
        <strong>Aria Web</strong>
        <span className="text-sm text-slate-600">single workspace</span>
      </header>
      <NewTaskPanel onCreateTask={handleCreateTask} busy={busy} />
      {error ? (
        <div role="alert" className="border-b border-danger/30 bg-red-50 px-4 py-2 text-sm text-danger">
          {error}
        </div>
      ) : null}
      <div className="grid min-h-[calc(100vh-7.75rem)] grid-cols-[18rem_minmax(0,1fr)_24rem]">
        <nav aria-label="Node flow" className="border-r border-line bg-panel p-3">
          <span className="text-xs font-semibold uppercase text-slate-500">Flow</span>
        </nav>
        <main className="p-4">
          <h1 className="text-xl font-semibold">Node Workspace</h1>
        </main>
        <aside className="border-l border-line bg-white p-3">
          <span className="text-sm font-medium">Evidence</span>
          {store.snapshot.projection?.active_task_id ? (
            <p className="mt-2 text-xs text-slate-500">{store.snapshot.projection.active_task_id}</p>
          ) : null}
        </aside>
      </div>
    </div>
  );
}
