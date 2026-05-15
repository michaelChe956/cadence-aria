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
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
    >
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">仓库</h2>
        <GitBranch className="h-4 w-4 text-[var(--aria-ink-muted)]" />
      </div>
      <dl className="grid gap-3 text-sm">
        <RepositoryRow label="当前项目" value={project?.name ?? "未选择"} />
        <RepositoryRow label="项目 ID" value={project?.project_id ?? "-"} mono />
        <RepositoryRow label="Repo path" value="未绑定仓库" mono />
        <RepositoryRow label="Repo hash" value="-" mono />
        <RepositoryRow label="Runtime root" value="-" mono />
        <RepositoryRow label="Issue" value={String(issueCount)} mono />
      </dl>
    </section>
  );
}

function RepositoryRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div>
      <dt className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
        {label}
      </dt>
      <dd
        className={
          mono
            ? "mt-1 break-all font-mono text-xs font-medium text-[var(--aria-ink)]"
            : "mt-1 break-words text-sm font-medium text-[var(--aria-ink)]"
        }
      >
        {value}
      </dd>
    </div>
  );
}
