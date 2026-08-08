import type { ReactNode } from "react";
import type { GroupFinalReadinessSnapshot, GroupFinalReadinessUnit, ReviewFinding } from "../../api/types";

export function GroupFinalReadinessPanel({
  readiness,
}: {
  readiness: GroupFinalReadinessSnapshot;
}) {
  return (
    <section
      data-testid="group-final-readiness-panel"
      className="space-y-3"
      aria-label="人工组最终确认凭据"
    >
      <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="text-sm font-semibold text-[var(--aria-ink)]">人工组最终确认</h3>
          <ReadinessStatusBadge status={readiness.status} />
        </div>
        <div className="mt-1 font-mono text-xs text-[var(--aria-ink-muted)]">
          attempt: {readiness.attempt_id}
        </div>
      </div>

      {readiness.diagnostics.length > 0 ? (
        <section
          data-testid="group-final-readiness-diagnostics"
          className="space-y-2 rounded-md border border-amber-200 bg-amber-50 p-3"
          aria-label="最终确认诊断"
        >
          <h4 className="text-xs font-semibold text-amber-900">确认前诊断</h4>
          <ul className="space-y-1.5">
            {readiness.diagnostics.map((diagnostic, index) => (
              <li
                key={`${diagnostic.kind}-${diagnostic.unit_id ?? "group"}-${index}`}
                className="text-xs text-amber-900"
              >
                <span className="mr-1 font-mono text-[11px]">[{diagnostic.kind}]</span>
                {diagnostic.message}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {readiness.units.map((unit) => (
        <ReadinessUnitEvidence key={unit.unit_id} unit={unit} />
      ))}
    </section>
  );
}

function ReadinessUnitEvidence({ unit }: { unit: GroupFinalReadinessUnit }) {
  const noObservableGitIncrement = unit.empty_observation || unit.commit_shas.length === 0;
  const commitRange = `${unit.start_commit ?? "-"}..${unit.completion_commit ?? "-"}`;
  const findings = unit.review_findings ?? [];

  return (
    <article className="space-y-3 rounded-md border border-[var(--aria-line)] bg-white p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="font-mono text-sm font-semibold text-[var(--aria-ink)]">
            {unit.logical_work_item_id}
          </h4>
          <div className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">unit: {unit.unit_id}</div>
        </div>
        {unit.unit_run_id ? (
          <span className="font-mono text-xs text-[var(--aria-ink-muted)]">
            run: {unit.unit_run_id}
          </span>
        ) : null}
      </div>

      <EvidenceSection title="Git 证据">
        <EvidenceRow label="Commit range" value={commitRange} mono />
        {noObservableGitIncrement ? (
          <div className="text-xs font-medium text-[var(--aria-ink-muted)]">无可观察 Git 增量</div>
        ) : (
          <EvidenceList label="Commit SHAs" values={unit.commit_shas} />
        )}
        <EvidenceRow label="Diff ref" value={unit.diff_ref} mono />
      </EvidenceSection>

      <EvidenceSection title="独立代码审查">
        <EvidenceRow label="Review report" value={unit.code_review_report_id ?? "-"} mono />
        <EvidenceRow label="Verdict" value={unit.review_verdict ?? "-"} />
        <EvidenceRow label="Summary" value={unit.review_summary ?? "-"} />
        {findings.length > 0 ? (
          <div className="space-y-2">
            <div className="text-xs font-semibold text-[var(--aria-ink)]">Findings</div>
            {findings.map((finding, index) => (
              <ReadinessFindingItem
                key={`${finding.file_path ?? "global"}-${finding.line ?? index}-${index}`}
                finding={finding}
              />
            ))}
          </div>
        ) : (
          <EvidenceRow label="Findings" value="无" />
        )}
        <EvidenceRow label="Raw ref" value={unit.review_raw_provider_output_ref ?? "-"} mono />
      </EvidenceSection>

      <EvidenceSection title="交接与计划">
        <EvidenceRow label="Handoff revision" value={unit.handoff_revision_id ?? "-"} mono />
        <EvidenceRow label="Plan revision" value={unit.plan_revision_id ?? "-"} mono />
      </EvidenceSection>
    </article>
  );
}

function EvidenceSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-1.5 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
      <h5 className="text-xs font-semibold text-[var(--aria-ink)]">{title}</h5>
      {children}
    </section>
  );
}

function EvidenceRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] gap-2 text-xs">
      <span className="text-[var(--aria-ink-muted)]">{label}</span>
      <span className={mono ? "break-all font-mono text-[var(--aria-ink)]" : "break-words text-[var(--aria-ink)]"}>
        {value}
      </span>
    </div>
  );
}

function EvidenceList({ label, values }: { label: string; values: string[] }) {
  return (
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] gap-2 text-xs">
      <span className="text-[var(--aria-ink-muted)]">{label}</span>
      <ul className="space-y-0.5 font-mono text-[var(--aria-ink)]">
        {values.map((value) => (
          <li key={value} className="break-all">
            {value}
          </li>
        ))}
      </ul>
    </div>
  );
}

function ReadinessFindingItem({ finding }: { finding: ReviewFinding }) {
  const location = finding.file_path
    ? finding.line
      ? `${finding.file_path}:${finding.line}`
      : finding.file_path
    : "global";
  return (
    <div className="rounded border border-[var(--aria-line)] bg-white p-2 text-xs">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5 font-semibold">
          {finding.severity}
        </span>
        <span className="truncate font-mono text-[var(--aria-ink-muted)]">{location}</span>
      </div>
      <div className="mt-1 text-[var(--aria-ink)]">{finding.message}</div>
      {finding.required_action ? (
        <div className="mt-1 text-[var(--aria-ink-muted)]">{finding.required_action}</div>
      ) : null}
    </div>
  );
}

function ReadinessStatusBadge({ status }: { status: GroupFinalReadinessSnapshot["status"] }) {
  const className =
    status === "complete"
      ? "border-emerald-200 bg-emerald-50 text-emerald-700"
      : "border-amber-200 bg-amber-50 text-amber-800";
  return (
    <span className={`rounded border px-1.5 py-0.5 text-xs font-semibold ${className}`}>
      {status}
    </span>
  );
}
