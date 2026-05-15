import type { Project } from "../../api/types";

export type ProjectSwitcherProps = {
  projects: Project[];
  selectedProjectId: string | null;
  disabled?: boolean;
  onSelectProject: (projectId: string) => void;
};

export function ProjectSwitcher({
  projects,
  selectedProjectId,
  disabled = false,
  onSelectProject,
}: ProjectSwitcherProps) {
  return (
    <label className="flex min-w-0 items-center gap-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
      <span className="shrink-0">项目</span>
      <select
        aria-label="选择项目"
        value={selectedProjectId ?? ""}
        disabled={disabled || projects.length === 0}
        onChange={(event) => onSelectProject(event.target.value)}
        className="h-8 w-48 max-w-[52vw] rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-2 text-sm font-medium text-[var(--aria-ink)] outline-none focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
      >
        {projects.length === 0 ? <option value="">暂无项目</option> : null}
        {projects.map((project) => (
          <option key={project.project_id} value={project.project_id}>
            {project.name}
          </option>
        ))}
      </select>
    </label>
  );
}
