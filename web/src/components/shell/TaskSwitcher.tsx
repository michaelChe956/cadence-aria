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
      <span className="shrink-0 text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
        继续任务
      </span>
      <select
        aria-label="继续任务"
        value={activeTaskId ?? ""}
        onChange={(event) => onSelectTask(event.target.value)}
        className="h-8 min-w-0 max-w-[18rem] truncate rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-2 text-sm font-semibold text-[var(--aria-ink)] outline-none transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
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
