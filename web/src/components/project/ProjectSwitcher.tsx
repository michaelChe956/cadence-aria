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
    <label className="text-xs font-black text-slate-700">
      项目
      <select
        aria-label="选择项目"
        value={selectedProjectId ?? ""}
        disabled={disabled || projects.length === 0}
        onChange={(event) => onSelectProject(event.target.value)}
        className="ml-2 h-9 min-w-48 rounded-lg border-2 border-slate-200 bg-white px-3 text-sm font-bold text-slate-950 shadow-inner shadow-slate-200/70 outline-none focus-visible:border-cyan-500 focus-visible:ring-4 focus-visible:ring-cyan-200 disabled:bg-slate-100 disabled:text-slate-500"
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
