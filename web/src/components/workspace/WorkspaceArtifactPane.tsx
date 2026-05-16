import { FileText, GitPullRequestDraft, Settings2 } from "lucide-react";
import type { WorkspaceSession } from "../../api/types";

export function WorkspaceArtifactPane({ session }: { session: WorkspaceSession }) {
  return (
    <section
      role="region"
      aria-label="Workspace 产物"
      className="min-h-0 overflow-auto bg-[var(--aria-panel-muted)] p-3"
    >
      <div className="space-y-3">
        <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3">
          <h3 className="mb-2 flex items-center text-sm font-semibold text-[var(--aria-ink)]">
            <FileText className="mr-1.5 h-4 w-4 text-[var(--aria-primary)]" />
            产物
          </h3>
          <dl className="space-y-1 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            <div>
              <dt className="inline">session</dt>
              <dd className="ml-2 inline text-[var(--aria-ink)]">{session.workspace_session_id}</dd>
            </div>
            <div>
              <dt className="inline">entity</dt>
              <dd className="ml-2 inline text-[var(--aria-ink)]">{session.entity_id}</dd>
            </div>
            <div>
              <dt className="inline">issue</dt>
              <dd className="ml-2 inline text-[var(--aria-ink)]">{session.issue_id}</dd>
            </div>
          </dl>
        </div>

        <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3">
          <h3 className="mb-2 flex items-center text-sm font-semibold text-[var(--aria-ink)]">
            <GitPullRequestDraft className="mr-1.5 h-4 w-4 text-[var(--aria-primary)]" />
            Provider
          </h3>
          <dl className="space-y-1 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            <div>
              <dt className="inline">author</dt>
              <dd className="ml-2 inline text-[var(--aria-ink)]">{session.author_provider}</dd>
            </div>
            <div>
              <dt className="inline">reviewer</dt>
              <dd className="ml-2 inline text-[var(--aria-ink)]">{session.reviewer_provider}</dd>
            </div>
            <div>
              <dt className="inline">review</dt>
              <dd className="ml-2 inline text-[var(--aria-ink)]">{session.review_rounds}</dd>
            </div>
          </dl>
        </div>

        <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3">
          <h3 className="mb-2 flex items-center text-sm font-semibold text-[var(--aria-ink)]">
            <Settings2 className="mr-1.5 h-4 w-4 text-[var(--aria-primary)]" />
            配置
          </h3>
          <div className="flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            {session.superpowers_enabled ? (
              <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                superpowers
              </span>
            ) : null}
            {session.openspec_enabled ? (
              <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                openspec
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </section>
  );
}
