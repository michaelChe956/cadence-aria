export function TaskSwitcher({
  tasks,
  activeTaskId,
  onSelectTask,
}: {
  tasks: Array<{ task_id: string; change_id: string | null; phase: string | null }>;
  activeTaskId: string | null;
  onSelectTask: (taskId: string) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="text-slate-500">继续任务</span>
      <select
        aria-label="继续任务"
        value={activeTaskId ?? ""}
        onChange={(event) => onSelectTask(event.target.value)}
        className="rounded-md border border-line bg-white px-2 py-1"
      >
        {tasks.length === 0 ? <option value="">no tasks</option> : null}
        {tasks.map((task) => (
          <option key={task.task_id} value={task.task_id}>
            {task.task_id} · {task.change_id ?? "no change"} · {task.phase ?? "unknown"}
          </option>
        ))}
      </select>
    </label>
  );
}
