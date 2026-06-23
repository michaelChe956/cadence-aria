import type {
  ValidatorFindingDto,
  WorkItemBatchStatePayload,
  WorkItemDraftCandidate,
  WorkItemDraftRecord,
  WorkItemDraftVerificationCommand,
  WorkItemDraftVerificationPlan,
  WorkItemPlanArtifactPayload,
  WorkItemPlanOutline,
  WorkItemPlanOutlineItem,
} from "../../api/types";

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
        <OutlineArtifact artifact={artifact.payload} />
      ) : null}

      {artifact.type === "context_blocker" ? (
        <section className="space-y-3">
          <Header title="Context Blocker" meta={`${artifact.payload.context_blockers.length}`} />
          <Paragraph>{artifact.payload.exploration_summary}</Paragraph>
          {artifact.payload.context_blockers.map((blocker) => (
            <KeyValue key={blocker.code} label={blocker.code} value={blocker.message} />
          ))}
        </section>
      ) : null}

      {artifact.type === "draft_candidate" ? (
        <section className="space-y-3">
          <Header
            title="Work Item Draft"
            meta={`${artifact.payload.draft_record.outline_id} / ${artifact.payload.draft_record.status}`}
          />
          <WorkItemDraftCard
            record={artifact.payload.draft_record}
            canAccept={artifact.payload.can_accept}
          />
          <ValidatorFindings findings={artifact.payload.validator_findings} />
        </section>
      ) : null}

      {artifact.type === "batch_state" ? <BatchArtifact payload={artifact.payload} /> : null}

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
            <div className="rounded-md border border-[var(--aria-line)] p-3">
              <div className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">Before</div>
              <pre className="whitespace-pre-wrap break-words text-xs text-[var(--aria-ink)]">
                {JSON.stringify({ materialized_work_items: [], child_sessions: [] }, null, 2)}
              </pre>
            </div>
            <div className="rounded-md border border-[var(--aria-line)] p-3">
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
          <ValidatorFindings findings={artifact.payload.validator_findings} />
        </section>
      ) : null}
    </div>
  );
}

function OutlineArtifact({
  artifact,
}: {
  artifact: Extract<WorkItemPlanArtifactPayload, { type: "outline_candidate" }>["payload"];
}) {
  const outline = artifact.outline;
  const items = outlineItems(outline);
  return (
    <section className="space-y-4">
      <Header title="Work Item Plan Outline" meta={artifact.current_generation_round_id ?? "--"} />
      <Paragraph>{outline.strategy_summary}</Paragraph>
      <div className="grid gap-2 sm:grid-cols-2">
        <KeyValue label="items" value={String(items.length)} />
        <KeyValue label="status" value={outline.status ?? "--"} />
      </div>
      {outline.handoff_strategy ? (
        <ReadableBlock title="Handoff strategy" content={outline.handoff_strategy} />
      ) : null}
      {outline.risks.length > 0 ? <BulletList title="Risks" items={outline.risks} /> : null}
      {outline.dependency_graph.length > 0 ? (
        <BulletList
          title="Dependencies"
          items={outline.dependency_graph.map((edge) => {
            if ("from_outline_id" in edge && "to_outline_id" in edge) {
              return `${edge.from_outline_id} -> ${edge.to_outline_id}`;
            }
            return `${edge.from_work_item_id} -> ${edge.to_work_item_id}`;
          })}
        />
      ) : null}
      <div className="space-y-3">
        {items.map((item, index) => (
          <WorkItemOutlineCard key={item.outline_id} item={item} index={index} />
        ))}
      </div>
      <ValidatorFindings findings={artifact.validator_findings} />
    </section>
  );
}

function WorkItemOutlineCard({ item, index }: { item: WorkItemPlanOutlineItem; index: number }) {
  return (
    <article className="rounded-md border border-[var(--aria-line)] bg-white p-3">
      <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
            #{index + 1} · {item.outline_id}
          </div>
          <h3 className="mt-1 break-words text-sm font-semibold text-[var(--aria-ink)]">
            {item.title}
          </h3>
        </div>
        <span className="rounded border border-[var(--aria-line)] px-2 py-0.5 text-xs text-[var(--aria-ink-muted)]">
          {item.kind}
        </span>
      </div>
      {item.goal ? <ReadableBlock title="Goal" content={item.goal} /> : null}
      <DetailLists
        rows={[
          ["Scope", item.scope ?? []],
          ["Non-goals", item.non_goals ?? []],
          ["Depends on", item.depends_on ?? item.depends_on_outline_ids ?? []],
          ["Required handoff", item.required_handoff_from_outline_ids ?? []],
          ["Write scopes", item.exclusive_write_scopes],
          ["Forbidden scopes", item.forbidden_write_scopes],
          [
            "Verification",
            item.verification_intent ??
              (item.verification_strategy ? [item.verification_strategy] : []),
          ],
        ]}
      />
      {item.handoff_notes ? <ReadableBlock title="Handoff notes" content={item.handoff_notes} /> : null}
      {item.risk_notes?.length ? <BulletList title="Risk notes" items={item.risk_notes} /> : null}
    </article>
  );
}

function BatchArtifact({ payload }: { payload: WorkItemBatchStatePayload }) {
  return (
    <section className="space-y-4">
      <Header title="Work Item Batch" meta={payload.batch_status} />
      <KeyValue label="batch" value={payload.batch_id} />
      <KeyValue label="queue" value={payload.queue.join(" -> ") || "--"} />
      <KeyValue label="drafts" value={String(payload.draft_records.length)} />
      {payload.failure_summary.length > 0 ? (
        <BulletList
          title="Failures"
          items={payload.failure_summary.map(
            (failure) => `${failure.outline_id} / ${failure.draft_id} / ${failure.status}`,
          )}
        />
      ) : null}
      <div className="space-y-3">
        {payload.draft_records.map((record) => (
          <WorkItemDraftCard key={record.draft_id} record={record} />
        ))}
      </div>
    </section>
  );
}

function WorkItemDraftCard({
  record,
  canAccept,
}: {
  record: WorkItemDraftRecord;
  canAccept?: boolean;
}) {
  const candidate = record.candidate;
  return (
    <article className="rounded-md border border-[var(--aria-line)] bg-white p-3">
      <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
            {record.outline_id} · {record.draft_id}
          </div>
          <h3 className="mt-1 break-words text-sm font-semibold text-[var(--aria-ink)]">
            {candidate.title}
          </h3>
        </div>
        <span className="rounded border border-[var(--aria-line)] px-2 py-0.5 text-xs text-[var(--aria-ink-muted)]">
          {candidate.kind} / {record.status}
        </span>
      </div>
      {candidate.goal ? <ReadableBlock title="Goal" content={candidate.goal} /> : null}
      <ReadableBlock title="Implementation context" content={candidate.implementation_context} />
      <DetailLists
        rows={[
          ["Depends on", candidate.depends_on_outline_ids],
          ["Required handoff", candidate.required_handoff_from_outline_ids],
          ["Write scopes", candidate.exclusive_write_scopes],
          ["Forbidden scopes", candidate.forbidden_write_scopes],
        ]}
      />
      <VerificationPlan plan={candidate.verification_plan} />
      <ReadableBlock title="Handoff summary" content={candidate.handoff_summary} />
      {typeof canAccept === "boolean" ? (
        <KeyValue label="can accept" value={canAccept ? "yes" : "no"} />
      ) : null}
    </article>
  );
}

function VerificationPlan({ plan }: { plan: WorkItemDraftVerificationPlan }) {
  const commands = plan.commands
    .map(commandLabel)
    .filter((command): command is string => Boolean(command));
  const gates = plan.required_gates
    .map((gate) => (typeof gate === "string" ? gate : gate.name ?? gate.gate_id ?? gate.description))
    .filter((gate): gate is string => Boolean(gate));
  const manualChecks = plan.manual_checks
    .map((check) => check.label ?? check.instructions)
    .filter((check): check is string => Boolean(check));

  return (
    <div className="mt-3 space-y-2">
      <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">Verification</div>
      <DetailLists
        rows={[
          ["Commands", commands],
          ["Required gates", gates],
          ["Manual checks", manualChecks],
          ["Risk notes", plan.risk_notes ?? []],
        ]}
      />
    </div>
  );
}

function ValidatorFindings({ findings }: { findings: ValidatorFindingDto[] }) {
  if (findings.length === 0) {
    return null;
  }
  return (
    <div className="space-y-2">
      <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">Validator findings</div>
      {findings.map((finding, index) => (
        <div
          key={`${finding.code}-${index}`}
          className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800"
        >
          <div className="font-semibold">
            {finding.severity ?? finding.level} / {finding.code ?? finding.finding_id}
          </div>
          <div className="mt-1">{finding.message}</div>
        </div>
      ))}
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

function Paragraph({ children }: { children: string }) {
  return <p className="break-words text-sm leading-6 text-[var(--aria-ink)]">{children}</p>;
}

function ReadableBlock({ title, content }: { title: string; content: string }) {
  if (!content.trim()) {
    return null;
  }
  return (
    <div className="mt-3">
      <div className="mb-1 text-xs font-semibold text-[var(--aria-ink-muted)]">{title}</div>
      <div className="whitespace-pre-wrap break-words text-sm leading-6 text-[var(--aria-ink)]">
        {content}
      </div>
    </div>
  );
}

function BulletList({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) {
    return null;
  }
  return (
    <div className="space-y-1">
      <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">{title}</div>
      <ul className="list-disc space-y-1 pl-5 text-sm leading-6 text-[var(--aria-ink)]">
        {items.map((item, index) => (
          <li key={`${item}-${index}`} className="break-words">
            {item}
          </li>
        ))}
      </ul>
    </div>
  );
}

function DetailLists({ rows }: { rows: Array<[string, string[] | undefined]> }) {
  return (
    <div className="mt-3 grid gap-2">
      {rows.map(([label, values]) => (
        <KeyValue
          key={label}
          label={label}
          value={values && values.length > 0 ? values.join(", ") : "--"}
        />
      ))}
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

function outlineItems(outline: WorkItemPlanOutline) {
  return outline.work_item_outlines ?? outline.work_items ?? [];
}

function commandLabel(command: WorkItemDraftVerificationCommand) {
  return (
    command.command ??
    command.label ??
    command.description ??
    command.id ??
    null
  );
}
