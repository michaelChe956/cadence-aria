import { useState } from "react";
import type {
  ValidatorFindingDto,
  WorkItemBatchStatePayload,
  WorkItemDraftRecord,
  WorkItemDraftVerificationCommand,
  WorkItemDraftVerificationPlan,
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
  WorkItemPlanOutline,
  WorkItemPlanOutlineItem,
} from "../../api/types";
import { normalizeDisplayText } from "../chat-workspace/text-display";
import { MonacoViewer } from "../shared/MonacoViewer";

export type WorkItemPlanArtifactTab = "overview" | "outline" | "drafts" | "diff" | "review" | "json";

export function WorkItemPlanArtifactTabContent({
  artifact,
  activeTab,
  versions,
  selectedVersion,
}: {
  artifact: WorkItemPlanArtifactPayload;
  activeTab: WorkItemPlanArtifactTab;
  versions: WorkItemPlanArtifactVersion[];
  selectedVersion: number | null;
}) {
  if (activeTab === "json") {
    return (
      <div className="h-[420px] overflow-hidden rounded-md border border-[var(--aria-line)]">
        <MonacoViewer value={JSON.stringify(artifact, null, 2)} language="json" height="100%" />
      </div>
    );
  }

  if (activeTab === "review") {
    return <ReviewTab artifact={artifact} />;
  }

  if (activeTab === "diff") {
    return (
      <DiffTab
        artifact={artifact}
        versions={versions}
        selectedVersion={selectedVersion}
      />
    );
  }

  if (activeTab === "overview") {
    return <OverviewTab artifact={artifact} />;
  }

  if (activeTab === "outline") {
    return artifact.type === "outline_candidate" ? (
      <OutlineArtifact artifact={artifact.payload} />
    ) : (
      <EmptyTab message="当前 artifact 不包含 Outline。" />
    );
  }

  if (activeTab === "drafts") {
    return <DraftsTab artifact={artifact} />;
  }

  return null;
}

function DiffTab({
  artifact,
  versions,
  selectedVersion,
}: {
  artifact: WorkItemPlanArtifactPayload;
  versions: WorkItemPlanArtifactVersion[];
  selectedVersion: number | null;
}) {
  const compareArtifact =
    versions.find((version) => version.version === selectedVersion)?.artifact ?? artifact;
  const baseArtifact = latestComparableArtifact(versions, compareArtifact, selectedVersion);
  const rows = baseArtifact
    ? diffWorkItemPlanArtifacts(baseArtifact, compareArtifact)
    : [];

  return (
    <section
      data-testid="work-item-diff-tab"
      className="space-y-3 rounded-md border border-[var(--aria-line)] bg-white p-3"
    >
      {baseArtifact && rows.length > 0 ? (
        rows.map((row) => (
          <div
            key={row.field}
            className="grid gap-2 rounded border border-[var(--aria-line)] p-3 text-sm md:grid-cols-[12rem_minmax(0,1fr)_minmax(0,1fr)]"
          >
            <div className="font-mono text-xs font-semibold text-[var(--aria-ink)]">
              {row.field}
            </div>
            <div className="min-w-0 break-words text-red-700">{row.before || "--"}</div>
            <div className="min-w-0 break-words text-emerald-700">{row.after || "--"}</div>
          </div>
        ))
      ) : (
        <p className="text-sm text-[var(--aria-ink-muted)]">
          暂无可比较的 Outline/Draft 版本
        </p>
      )}
    </section>
  );
}

function OverviewTab({ artifact }: { artifact: WorkItemPlanArtifactPayload }) {
  if (artifact.type === "compile_report") {
    return <CompileReportArtifact artifact={artifact.payload} />;
  }
  if (artifact.type === "context_blocker") {
    return <ContextBlockerArtifact artifact={artifact.payload} />;
  }
  if (artifact.type === "batch_state") {
    return <BatchArtifact payload={artifact.payload} />;
  }
  if (artifact.type === "draft_candidate") {
    return <DraftsTab artifact={artifact} />;
  }
  return <OutlineArtifact artifact={artifact.payload} />;
}

function DraftsTab({ artifact }: { artifact: WorkItemPlanArtifactPayload }) {
  if (artifact.type === "draft_candidate") {
    return (
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
    );
  }

  if (artifact.type === "batch_state") {
    const selectedRecord = artifact.payload.draft_records[0] ?? null;
    return (
      <section className="space-y-4">
        <Header title="Work Item Drafts" meta={artifact.payload.batch_status} />
        <KeyValue label="batch" value={artifact.payload.batch_id} />
        <KeyValue label="queue" value={artifact.payload.queue.join(" -> ") || "--"} />
        {artifact.payload.failure_summary.length > 0 ? (
          <BulletList
            title="Failures"
            items={artifact.payload.failure_summary.map(
              (failure) => `${failure.outline_id} / ${failure.draft_id} / ${failure.status}`,
            )}
          />
        ) : null}
        <div
          data-testid="work-item-draft-list"
          className="grid gap-2 rounded-md border border-[var(--aria-line)] bg-white p-2 md:grid-cols-2"
        >
          {artifact.payload.draft_records.map((record) => (
            <div
              key={record.draft_id}
              className="rounded border border-[var(--aria-line)] px-3 py-2 text-xs"
            >
              <div className="font-mono font-semibold text-[var(--aria-ink)]">
                {record.draft_id}
              </div>
              <div className="mt-1 text-[var(--aria-ink-muted)]">
                {record.outline_id} / {record.status}
              </div>
            </div>
          ))}
        </div>
        {selectedRecord ? (
          <div data-testid="work-item-draft-detail">
            <WorkItemDraftCard record={selectedRecord} />
          </div>
        ) : null}
      </section>
    );
  }

  return <EmptyTab message="当前 artifact 不包含 Draft。" />;
}

function ReviewTab({ artifact }: { artifact: WorkItemPlanArtifactPayload }) {
  const findings = artifactFindings(artifact);
  return (
    <section
      data-testid="work-item-review-tab"
      className="space-y-3 rounded-md border border-[var(--aria-line)] bg-white p-3"
    >
      <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
        Blocking findings
      </div>
      {findings.length > 0 ? (
        <ValidatorFindings findings={findings} />
      ) : (
        <p className="text-sm text-[var(--aria-ink-muted)]">暂无 findings</p>
      )}
      {artifact.type === "context_blocker" ? (
        <ContextBlockerArtifact artifact={artifact.payload} />
      ) : null}
    </section>
  );
}

function EmptyTab({ message }: { message: string }) {
  return (
    <section className="rounded-md border border-[var(--aria-line)] bg-white p-3 text-sm text-[var(--aria-ink-muted)]">
      {message}
    </section>
  );
}

function ContextBlockerArtifact({
  artifact,
}: {
  artifact: Extract<WorkItemPlanArtifactPayload, { type: "context_blocker" }>["payload"];
}) {
  return (
    <section className="space-y-3">
      <Header title="Context Blocker" meta={`${artifact.context_blockers.length}`} />
      <Paragraph>{artifact.exploration_summary}</Paragraph>
      {artifact.context_blockers.map((blocker) => (
        <KeyValue key={blocker.code} label={blocker.code} value={blocker.message} />
      ))}
    </section>
  );
}

function CompileReportArtifact({
  artifact,
}: {
  artifact: Extract<WorkItemPlanArtifactPayload, { type: "compile_report" }>["payload"];
}) {
  return (
    <section className="space-y-3">
      <Header title="Compile Report" meta={artifact.status} />
      <KeyValue label="compile" value={artifact.compile_id} />
      <KeyValue label="commit state" value={artifact.plan_commit_state} />
      <KeyValue label="work items" value={artifact.work_item_ids.join(", ") || "--"} />
      <KeyValue
        label="verification plans"
        value={artifact.verification_plan_ids.join(", ") || "--"}
      />
      <KeyValue label="child sessions" value={artifact.child_session_ids.join(", ") || "--"} />
      <ValidatorFindings findings={artifact.validator_findings} />
    </section>
  );
}

function artifactFindings(artifact: WorkItemPlanArtifactPayload) {
  switch (artifact.type) {
    case "outline_candidate":
      return artifact.payload.validator_findings;
    case "draft_candidate":
      return artifact.payload.validator_findings;
    case "compile_report":
      return artifact.payload.validator_findings;
    default:
      return [];
  }
}

export function groupWorkItemPlanArtifactVersions(versions: WorkItemPlanArtifactVersion[]) {
  const groups: Array<{
    key: string;
    label: string;
    versions: WorkItemPlanArtifactVersion[];
  }> = [];
  for (const version of versions) {
    const group = versionGroup(version.artifact ?? null);
    let existing = groups.find((item) => item.key === group.key);
    if (!existing) {
      existing = { ...group, versions: [] };
      groups.push(existing);
    }
    existing.versions.push(version);
  }
  return groups;
}

function versionGroup(artifact: WorkItemPlanArtifactPayload | null) {
  switch (artifact?.type) {
    case "outline_candidate":
      return { key: "outline", label: "Outline" };
    case "draft_candidate":
    case "batch_state":
      return { key: "drafts", label: "Draft" };
    case "compile_report":
      return { key: "compile", label: "Final Compile" };
    case "context_blocker":
      return { key: "blockers", label: "Blockers" };
    default:
      return { key: "unknown", label: "Other" };
  }
}

function latestComparableArtifact(
  versions: WorkItemPlanArtifactVersion[],
  compareArtifact: WorkItemPlanArtifactPayload,
  selectedVersion: number | null,
) {
  const ordered = [...versions]
    .filter((version) => version.artifact && version.artifact !== compareArtifact)
    .filter((version) => selectedVersion === null || version.version < selectedVersion)
    .sort((left, right) => right.version - left.version);
  return ordered.find((version) =>
    version.artifact ? comparableArtifacts(version.artifact, compareArtifact) : false,
  )?.artifact ?? null;
}

function comparableArtifacts(
  base: WorkItemPlanArtifactPayload,
  compare: WorkItemPlanArtifactPayload,
) {
  if (base.type !== compare.type) {
    return false;
  }
  if (base.type === "draft_candidate" && compare.type === "draft_candidate") {
    return base.payload.draft_record.outline_id === compare.payload.draft_record.outline_id;
  }
  return base.type === "outline_candidate";
}

function diffWorkItemPlanArtifacts(
  base: WorkItemPlanArtifactPayload,
  compare: WorkItemPlanArtifactPayload,
) {
  if (base.type === "outline_candidate" && compare.type === "outline_candidate") {
    return diffOutlineArtifacts(base.payload.outline, compare.payload.outline);
  }
  if (base.type === "draft_candidate" && compare.type === "draft_candidate") {
    return diffDraftArtifacts(base.payload.draft_record, compare.payload.draft_record);
  }
  return [];
}

function diffOutlineArtifacts(base: WorkItemPlanOutline, compare: WorkItemPlanOutline) {
  const rows: Array<{ field: string; before: string; after: string }> = [];
  const baseItems = new Map(outlineItems(base).map((item) => [item.outline_id, item]));
  for (const compareItem of outlineItems(compare)) {
    const baseItem = baseItems.get(compareItem.outline_id);
    if (!baseItem) {
      rows.push({
        field: `${compareItem.outline_id}.added`,
        before: "",
        after: compareItem.title,
      });
      continue;
    }
    pushDiffRow(
      rows,
      "title",
      baseItem.title,
      compareItem.title,
    );
    pushDiffRow(
      rows,
      "estimated_context_tokens",
      baseItem.estimated_context_tokens !== undefined
        ? String(baseItem.estimated_context_tokens)
        : "",
      compareItem.estimated_context_tokens !== undefined
        ? String(compareItem.estimated_context_tokens)
        : "",
    );
    pushDiffRow(
      rows,
      "session_fit",
      baseItem.session_fit ?? "",
      compareItem.session_fit ?? "",
    );
    pushDiffRow(
      rows,
      "exclusive_write_scopes",
      baseItem.exclusive_write_scopes,
      compareItem.exclusive_write_scopes,
    );
    pushDiffRow(
      rows,
      "forbidden_write_scopes",
      baseItem.forbidden_write_scopes,
      compareItem.forbidden_write_scopes,
    );
    pushDiffRow(
      rows,
      "verification_intent",
      baseItem.verification_intent ?? [],
      compareItem.verification_intent ?? [],
    );
  }
  return rows;
}

function diffDraftArtifacts(base: WorkItemDraftRecord, compare: WorkItemDraftRecord) {
  const rows: Array<{ field: string; before: string; after: string }> = [];
  pushDiffRow(rows, "title", base.candidate.title, compare.candidate.title);
  pushDiffRow(
    rows,
    "implementation_context",
    base.candidate.implementation_context,
    compare.candidate.implementation_context,
  );
  pushDiffRow(
    rows,
    "exclusive_write_scopes",
    base.candidate.exclusive_write_scopes,
    compare.candidate.exclusive_write_scopes,
  );
  pushDiffRow(
    rows,
    "verification.commands",
    base.candidate.verification_plan.commands.map(commandLabel).filter(isPresent),
    compare.candidate.verification_plan.commands.map(commandLabel).filter(isPresent),
  );
  pushDiffRow(
    rows,
    "handoff_summary",
    base.candidate.handoff_summary,
    compare.candidate.handoff_summary,
  );
  return rows;
}

function pushDiffRow(
  rows: Array<{ field: string; before: string; after: string }>,
  field: string,
  before: string | string[],
  after: string | string[],
) {
  const beforeValue = Array.isArray(before) ? before.join(", ") : before;
  const afterValue = Array.isArray(after) ? after.join(", ") : after;
  if (beforeValue === afterValue) {
    return;
  }
  rows.push({ field, before: beforeValue, after: afterValue });
}

function isPresent(value: string | null): value is string {
  return Boolean(value);
}

function OutlineArtifact({
  artifact,
}: {
  artifact: Extract<WorkItemPlanArtifactPayload, { type: "outline_candidate" }>["payload"];
}) {
  const [view, setView] = useState<"cards" | "source">("cards");
  const outline = artifact.outline;
  const items = outlineItems(outline);
  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <Header title="Work Item Plan Outline" meta={artifact.current_generation_round_id ?? "--"} />
        <div className="flex items-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-0.5 text-xs">
          <button
            type="button"
            data-testid="outline-view-cards"
            onClick={() => setView("cards")}
            className={`rounded px-2 py-1 ${
              view === "cards"
                ? "bg-white font-medium text-[var(--aria-ink)] shadow-sm"
                : "text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
            }`}
          >
            Cards
          </button>
          <button
            type="button"
            data-testid="outline-view-source"
            onClick={() => setView("source")}
            className={`rounded px-2 py-1 ${
              view === "source"
                ? "bg-white font-medium text-[var(--aria-ink)] shadow-sm"
                : "text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
            }`}
          >
            Source
          </button>
        </div>
      </div>
      {view === "source" ? (
        <div className="h-[400px] overflow-hidden rounded-md border border-[var(--aria-line)]">
          <MonacoViewer value={JSON.stringify(outline, null, 2)} language="json" height="100%" />
        </div>
      ) : (
        <>
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
          <div className="space-y-3" data-testid="work-item-outline-table">
            {items.map((item, index) => (
              <WorkItemOutlineCard key={item.outline_id} item={item} index={index} />
            ))}
          </div>
          <ValidatorFindings findings={artifact.validator_findings} />
        </>
      )}
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
          [
            "Budget",
            item.estimated_context_tokens
              ? [`${item.estimated_context_tokens} tokens`]
              : [],
          ],
          ["Session fit", item.session_fit ? [item.session_fit] : []],
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
  return (
    <p className="break-words text-sm leading-6 text-[var(--aria-ink)]">
      {normalizeDisplayText(children)}
    </p>
  );
}

function ReadableBlock({ title, content }: { title: string; content: string }) {
  const displayContent = normalizeDisplayText(content);
  if (!displayContent.trim()) {
    return null;
  }
  return (
    <div className="mt-3">
      <div className="mb-1 text-xs font-semibold text-[var(--aria-ink-muted)]">{title}</div>
      <div className="whitespace-pre-wrap break-words text-sm leading-6 text-[var(--aria-ink)]">
        {displayContent}
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
            {normalizeDisplayText(item)}
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
  const displayValue = normalizeDisplayText(value);
  return (
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] gap-3 text-sm">
      <span className="text-[var(--aria-ink-muted)]">{label}</span>
      <span className="min-w-0 break-words text-[var(--aria-ink)]">{displayValue}</span>
    </div>
  );
}

function outlineItems(outline: WorkItemPlanOutline) {
  return outline.work_item_outlines ?? outline.work_items ?? [];
}

export function workItemPlanArtifactLabel(
  artifact: WorkItemPlanArtifactPayload,
): string {
  switch (artifact.type) {
    case "outline_candidate":
      return `Outline · ${outlineItems(artifact.payload.outline).length} items`;
    case "draft_candidate":
      return `${artifact.payload.draft_record.outline_id} / ${artifact.payload.draft_record.draft_id}`;
    case "batch_state":
      return `Batch / ${artifact.payload.batch_id}`;
    case "compile_report":
      return `Final Compile / ${artifact.payload.compile_id}`;
    case "context_blocker":
      return `Blocker / ${artifact.payload.context_blockers.length}`;
    default:
      return "Artifact";
  }
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
