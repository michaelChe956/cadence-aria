import { FolderKanban, RefreshCw } from "lucide-react";
import type { Project } from "../../api/types";
import { ProjectSwitcher } from "./ProjectSwitcher";

export type ProjectTopBarProps = {
  projects: Project[];
  selectedProjectId: string | null;
  issueCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => void;
  onRefresh: () => void;
};

export function ProjectTopBar({
  projects,
  selectedProjectId,
  issueCount,
  busy,
  onSelectProject,
  onRefresh,
}: ProjectTopBarProps) {
  return (
    <header
      role="banner"
      className="sticky top-0 z-20 flex min-h-16 flex-wrap items-center justify-between gap-3 border-b-2 border-slate-200 bg-white/92 px-4 py-3 shadow-[0_8px_28px_rgba(15,23,42,0.08)] backdrop-blur md:px-6 lg:px-8"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-3">
        <div className="flex items-center gap-2">
          <FolderKanban className="h-5 w-5 text-cyan-700" />
          <strong className="text-lg text-[#241B2F]">Aria Web</strong>
        </div>
        <span className="rounded-lg border-2 border-cyan-200 bg-cyan-50 px-3 py-1 text-xs font-black text-cyan-950">
          项目工作台
        </span>
        <span className="rounded-lg border-2 border-slate-200 bg-white px-3 py-1 font-mono text-xs font-bold text-slate-700">
          Issue {issueCount}
        </span>
      </div>
      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
        <ProjectSwitcher
          projects={projects}
          selectedProjectId={selectedProjectId}
          disabled={busy}
          onSelectProject={onSelectProject}
        />
        <button
          type="button"
          onClick={onRefresh}
          disabled={busy}
          className="inline-flex h-9 items-center justify-center rounded-lg border-2 border-slate-200 bg-white px-3 text-sm font-black text-slate-700 shadow-[0_4px_0_rgba(15,23,42,0.08)] transition-colors hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200 disabled:bg-slate-100 disabled:text-slate-500 disabled:shadow-none"
        >
          <RefreshCw className="mr-1 h-4 w-4" />
          刷新
        </button>
      </div>
    </header>
  );
}
