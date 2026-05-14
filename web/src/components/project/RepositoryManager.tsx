import { GitBranch } from "lucide-react";
import type { Project } from "../../api/types";

export type RepositoryManagerProps = {
  project: Project | null;
  issueCount: number;
};

export function RepositoryManager({ project, issueCount }: RepositoryManagerProps) {
  return (
    <section
      role="region"
      aria-label="仓库面板"
      className="rounded-lg border-2 border-slate-200 bg-white p-4 shadow-[0_8px_0_rgba(15,23,42,0.06),0_18px_34px_rgba(15,23,42,0.08)]"
    >
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-lg font-black text-[#241B2F]">仓库</h2>
        <GitBranch className="h-5 w-5 text-slate-600" />
      </div>
      <dl className="grid gap-3 text-sm">
        <div>
          <dt className="text-xs font-black text-slate-500">当前项目</dt>
          <dd className="mt-1 font-bold text-slate-950">{project?.name ?? "未选择"}</dd>
        </div>
        <div>
          <dt className="text-xs font-black text-slate-500">项目 ID</dt>
          <dd className="mt-1 break-all font-mono text-xs font-bold text-slate-700">
            {project?.project_id ?? "-"}
          </dd>
        </div>
        <div>
          <dt className="text-xs font-black text-slate-500">Issue</dt>
          <dd className="mt-1 font-mono text-xs font-bold text-slate-700">{issueCount}</dd>
        </div>
      </dl>
    </section>
  );
}
