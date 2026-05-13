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
    <label className="flex min-w-0 flex-1 items-center justify-end gap-2 text-sm sm:flex-none">
      <span className="shrink-0 text-xs font-bold uppercase text-indigo-700">
        继续任务
      </span>
      <select
        aria-label="继续任务"
        value={activeTaskId ?? ""}
        onChange={(event) => onSelectTask(event.target.value)}
        className="min-w-0 max-w-[22rem] rounded-lg border-2 border-indigo-200 bg-white px-3 py-1.5 text-sm font-semibold text-indigo-950 shadow-[0_4px_0_rgba(129,140,248,0.18)] outline-none transition-colors hover:border-orange-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
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
