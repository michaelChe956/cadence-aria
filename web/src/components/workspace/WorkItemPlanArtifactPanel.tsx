import type { WorkItemPlanArtifactPayload } from "../../api/types";

export interface WorkItemPlanArtifactPanelProps {
  artifact: WorkItemPlanArtifactPayload | null;
  readonly?: boolean;
  className?: string;
}

export function WorkItemPlanArtifactPanel({
  artifact,
  readonly = false,
  className = "",
}: WorkItemPlanArtifactPanelProps) {
  if (!artifact) {
    return (
      <div
        data-testid="work-item-plan-artifact-panel"
        className={`min-h-0 overflow-auto p-4 text-sm text-[var(--aria-ink-muted)] ${className}`}
      >
        尚未生成 staged artifact
      </div>
    );
  }

  return (
    <div
      data-testid="work-item-plan-artifact-panel"
      className={`min-h-0 overflow-auto p-4 ${className}`}
    >
      {readonly ? (
        <div className="mb-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs font-semibold text-amber-800">
          只读历史
        </div>
      ) : null}

      {artifact.type === "outline_candidate" ? (
        <section className="space-y-3">
          <Header title="Outline" meta={artifact.payload.current_generation_round_id ?? "--"} />
          <p className="text-sm text-[var(--aria-ink)]">{artifact.payload.outline.strategy_summary}</p>
          <KeyValue label="items" value={String(artifact.payload.outline.work_items.length)} />
          <div className="divide-y divide-[var(--aria-line)] border-y border-[var(--aria-line)]">
            {artifact.payload.outline.work_items.map((item) => (
              <div key={item.outline_id} className="py-2 text-sm">
                <div className="font-semibold text-[var(--aria-ink)]">{item.title}</div>
                <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
                  {item.outline_id} / {item.kind}
                </div>
              </div>
            ))}
          </div>
        </section>
      ) : null}

      {artifact.type === "context_blocker" ? (
        <section className="space-y-3">
          <Header title="Context Blocker" meta={`${artifact.payload.context_blockers.length}`} />
          <p className="text-sm text-[var(--aria-ink)]">{artifact.payload.exploration_summary}</p>
          {artifact.payload.context_blockers.map((blocker) => (
            <KeyValue key={blocker.code} label={blocker.code} value={blocker.message} />
          ))}
        </section>
      ) : null}

      {artifact.type === "draft_candidate" ? (
        <section className="space-y-3">
          <Header
            title="Draft"
            meta={artifact.payload.draft_record.status}
          />
          <KeyValue label="outline" value={artifact.payload.draft_record.outline_id} />
          <KeyValue label="title" value={artifact.payload.draft_record.candidate.title} />
          <KeyValue
            label="write scopes"
            value={artifact.payload.draft_record.candidate.exclusive_write_scopes.join(", ") || "--"}
          />
          <KeyValue label="can accept" value={artifact.payload.can_accept ? "yes" : "no"} />
        </section>
      ) : null}

      {artifact.type === "batch_state" ? (
        <section className="space-y-3">
          <Header title="Batch" meta={artifact.payload.batch_status} />
          <KeyValue label="queue" value={artifact.payload.queue.join(" -> ") || "--"} />
          <KeyValue label="drafts" value={String(artifact.payload.draft_records.length)} />
          <KeyValue label="failures" value={String(artifact.payload.failure_summary.length)} />
          {artifact.payload.failure_summary.length > 0 ? (
            <div className="space-y-2">
              {artifact.payload.failure_summary.map((failure, index) => (
                <KeyValue
                  key={`${failure.draft_id}-${failure.outline_id}`}
                  label={`failure ${index + 1}`}
                  value={`${failure.outline_id} / ${failure.status}`}
                />
              ))}
            </div>
          ) : null}
        </section>
      ) : null}

      {artifact.type === "compile_report" ? (
        <section className="space-y-3">
          <Header title="Compile Report" meta={artifact.payload.status} />
          <KeyValue label="compile" value={artifact.payload.compile_id} />
          <KeyValue label="commit state" value={artifact.payload.plan_commit_state} />
          <KeyValue label="work items" value={artifact.payload.work_item_ids.join(", ") || "--"} />
          <KeyValue
            label="verification plans"
            value={artifact.payload.verification_plan_ids.join(", ") || "--"}
          />
          <KeyValue
            label="child sessions"
            value={artifact.payload.child_session_ids.join(", ") || "--"}
          />
          <div className="grid gap-3 md:grid-cols-2" data-testid="compile-report-before-after">
            <div className="rounded border border-[var(--aria-line)] p-3">
              <div className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">Before</div>
              <pre className="whitespace-pre-wrap break-words text-xs text-[var(--aria-ink)]">
                {JSON.stringify({ materialized_work_items: [], child_sessions: [] }, null, 2)}
              </pre>
            </div>
            <div className="rounded border border-[var(--aria-line)] p-3">
              <div className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">After</div>
              <pre className="whitespace-pre-wrap break-words text-xs text-[var(--aria-ink)]">
                {JSON.stringify(
                  {
                    work_item_ids: artifact.payload.work_item_ids,
                    verification_plan_ids: artifact.payload.verification_plan_ids,
                    child_session_ids: artifact.payload.child_session_ids,
                  },
                  null,
                  2,
                )}
              </pre>
            </div>
          </div>
        </section>
      ) : null}
    </div>
  );
}

function Header({ title, meta }: { title: string; meta: string }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] pb-2">
      <h2 className="truncate text-sm font-semibold text-[var(--aria-ink)]">{title}</h2>
      <span className="shrink-0 rounded border border-[var(--aria-line)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
        {meta}
      </span>
    </div>
  );
}

function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] gap-3 text-sm">
      <span className="text-[var(--aria-ink-muted)]">{label}</span>
      <span className="min-w-0 break-words text-[var(--aria-ink)]">{value}</span>
    </div>
  );
}
