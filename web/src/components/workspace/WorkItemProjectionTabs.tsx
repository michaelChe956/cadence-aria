import { useRef, useState } from "react";
import type {
  CoderWorkItemProjection,
  HumanPresentationRevision,
  PlanProjectionBundle,
  ProjectionValidationReport,
  ReviewerWorkItemProjection,
  SaveHumanPresentationRevisionMessage,
  WorkItemProjectionBundle,
  WorkItemProjectionTab,
  WorkItemRevisionHistoryDto,
} from "../../api/types";
import type { HumanPresentationSaveState } from "../../state/workspace-ws-store-types";
import { WorkItemContractFlow } from "./WorkItemContractFlow";
import { WorkItemPlanOverview } from "./WorkItemPlanOverview";

const TABS: Array<{ id: WorkItemProjectionTab; label: string }> = [
  { id: "overview", label: "Human Overview" },
  { id: "contract", label: "Contract Flow" },
  { id: "coder", label: "Coder" },
  { id: "reviewer", label: "Reviewer" },
  { id: "history", label: "History" },
];

export function WorkItemProjectionTabs({
  planProjection,
  workItemProjections,
  history,
  validation,
  presentations = {},
  presentationSaveStates = {},
  editable = false,
  onSavePresentation = () => undefined,
}: {
  planProjection: PlanProjectionBundle;
  workItemProjections: WorkItemProjectionBundle[];
  history: WorkItemRevisionHistoryDto | null;
  validation: ProjectionValidationReport | null;
  presentations?: Record<string, HumanPresentationRevision>;
  presentationSaveStates?: Record<string, HumanPresentationSaveState>;
  editable?: boolean;
  onSavePresentation?: (message: SaveHumanPresentationRevisionMessage) => void;
}) {
  const [activeTab, setActiveTab] = useState<WorkItemProjectionTab>("overview");
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);

  function selectTab(index: number) {
    const tab = TABS[index];
    if (!tab) {
      return;
    }
    setActiveTab(tab.id);
    tabRefs.current[index]?.focus();
  }

  return (
    <section className="min-w-0 space-y-3">
      <div
        role="tablist"
        aria-label="Work Item Plan projections"
        className="flex min-w-0 gap-1 overflow-x-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-1"
      >
        {TABS.map((tab, index) => (
          <button
            key={tab.id}
            ref={(node) => {
              tabRefs.current[index] = node;
            }}
            id={`work-item-projection-tab-${tab.id}`}
            type="button"
            role="tab"
            aria-selected={activeTab === tab.id}
            aria-controls={
              activeTab === tab.id
                ? `work-item-projection-panel-${tab.id}`
                : undefined
            }
            tabIndex={activeTab === tab.id ? 0 : -1}
            onClick={() => setActiveTab(tab.id)}
            onKeyDown={(event) => {
              if (event.key === "ArrowRight") {
                event.preventDefault();
                selectTab((index + 1) % TABS.length);
              } else if (event.key === "ArrowLeft") {
                event.preventDefault();
                selectTab((index - 1 + TABS.length) % TABS.length);
              } else if (event.key === "Home") {
                event.preventDefault();
                selectTab(0);
              } else if (event.key === "End") {
                event.preventDefault();
                selectTab(TABS.length - 1);
              }
            }}
            className={`h-9 shrink-0 cursor-pointer rounded px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] ${
              activeTab === tab.id
                ? "bg-white text-[var(--aria-ink)] shadow-sm"
                : "text-[var(--aria-ink-muted)] hover:bg-white hover:text-[var(--aria-ink)]"
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div
        id={`work-item-projection-panel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`work-item-projection-tab-${activeTab}`}
        className="min-w-0"
      >
        {activeTab === "overview" ? (
          <WorkItemPlanOverview
            projection={planProjection.human_group_projection}
            presentation={presentations[planProjection.id] ?? null}
            planProjectionBundleId={planProjection.id}
            workItemProjections={workItemProjections}
            presentations={presentations}
            presentationSaveStates={presentationSaveStates}
            editable={editable}
            onSavePresentation={onSavePresentation}
          />
        ) : null}
        {activeTab === "contract" ? (
          <WorkItemContractFlow projection={planProjection.human_group_projection} />
        ) : null}
        {activeTab === "coder" ? (
          <CoderProjectionView
            planProjection={planProjection}
            workItemProjections={workItemProjections}
          />
        ) : null}
        {activeTab === "reviewer" ? (
          <ReviewerProjectionView
            planProjection={planProjection}
            workItemProjections={workItemProjections}
            validation={validation}
          />
        ) : null}
        {activeTab === "history" ? <HistoryView history={history} /> : null}
      </div>
    </section>
  );
}

function CoderProjectionView({
  planProjection,
  workItemProjections,
}: {
  planProjection: PlanProjectionBundle;
  workItemProjections: WorkItemProjectionBundle[];
}) {
  const projectionsByLogicalId = new Map(
    workItemProjections.map((bundle) => [
      bundle.human_projection.logical_work_item_id,
      bundle,
    ]),
  );
  return (
    <section className="space-y-4" aria-labelledby="coder-projection-title">
      <header className="rounded-md border border-[var(--aria-line)] bg-white p-4">
        <h3 id="coder-projection-title" className="text-sm font-semibold text-[var(--aria-ink)]">
          Coder 执行协议
        </h3>
        <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
          当前展示已发布的 provider-neutral projection；运行时上下文不会在前端合成。
        </p>
      </header>
      <div className="space-y-3">
        {planProjection.coder_group_context.ordered_logical_work_item_ids.map(
          (logicalId) => {
            const bundle = projectionsByLogicalId.get(logicalId);
            return bundle ? (
              <CoderWorkItemCard key={bundle.id} bundle={bundle} />
            ) : (
              <MissingProjectionCard key={logicalId} logicalId={logicalId} />
            );
          },
        )}
      </div>
      <ProviderRenderingStatus role="Coder" />
    </section>
  );
}

function CoderWorkItemCard({ bundle }: { bundle: WorkItemProjectionBundle }) {
  const projection = bundle.coder_projection;
  return (
    <article className="min-w-0 rounded-md border border-[var(--aria-line)] bg-white p-4">
      <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
        <div>
          <h4 className="text-sm font-semibold text-[var(--aria-ink)]">
            {bundle.human_projection.logical_work_item_id} {bundle.human_projection.title}
          </h4>
          <p className="mt-1 text-sm text-[var(--aria-ink-muted)]">
            {projection.objective}
          </p>
        </div>
        <span className="break-all rounded border border-[var(--aria-line)] px-2 py-1 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {projection.work_item_revision_id}
        </span>
      </div>
      <ProjectionSummary projection={projection} />
      <details className="mt-3 rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
        <summary className="cursor-pointer text-xs font-semibold text-[var(--aria-ink)]">
          完整 provider-neutral payload
        </summary>
        <pre className="mt-3 max-w-full overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs leading-5 text-[var(--aria-ink-muted)]">
          {JSON.stringify(projection, null, 2)}
        </pre>
      </details>
    </article>
  );
}

function ProjectionSummary({ projection }: { projection: CoderWorkItemProjection }) {
  return (
    <div className="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
      <CountCard label="Inputs" count={projection.required_input_contracts.length} />
      <CountCard label="Tasks" count={projection.tasks.length} />
      <CountCard label="Acceptance" count={projection.acceptance_criteria.length} />
      <CountCard label="Checks" count={projection.verification_checks.length} />
    </div>
  );
}

function ProviderRenderingStatus({ role }: { role: "Coder" | "Reviewer" }) {
  return (
    <section
      aria-label={`${role} provider rendering`}
      className="rounded-md border border-[var(--aria-line)] bg-white p-4"
    >
      <h4 className="text-sm font-semibold text-[var(--aria-ink)]">Provider rendering</h4>
      <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
        {role} Renderer 需要真实 Execution Envelope；该运行时证据由 P5 接入。
      </p>
      <div className="mt-3 grid gap-2 sm:grid-cols-3">
        {['Codex', 'Claude Code', 'Fake'].map((provider) => (
          <div
            key={provider}
            className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
          >
            <div className="text-xs font-semibold text-[var(--aria-ink)]">{provider}</div>
            <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
              等待运行时 Envelope（P5）
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function ReviewerProjectionView({
  planProjection,
  workItemProjections,
  validation,
}: {
  planProjection: PlanProjectionBundle;
  workItemProjections: WorkItemProjectionBundle[];
  validation: ProjectionValidationReport | null;
}) {
  const bundlesByLogicalId = new Map(
    workItemProjections.map((bundle) => [
      bundle.human_projection.logical_work_item_id,
      bundle,
    ]),
  );
  return (
    <section className="space-y-4" aria-labelledby="reviewer-projection-title">
      <header className="rounded-md border border-[var(--aria-line)] bg-white p-4">
        <h3
          id="reviewer-projection-title"
          className="text-sm font-semibold text-[var(--aria-ink)]"
        >
          Reviewer 验证矩阵
        </h3>
        <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
          验证标准与失败路由来自已发布 projection，不包含未发生的运行时证据。
        </p>
      </header>
      {planProjection.reviewer_group_matrix.work_items.map((matrixEntry) => {
        const bundle = bundlesByLogicalId.get(matrixEntry.logical_work_item_id);
        return bundle ? (
          <ReviewerWorkItemCard
            key={bundle.id}
            logicalId={matrixEntry.logical_work_item_id}
            projection={bundle.reviewer_projection}
            refs={{
              criteria: matrixEntry.criterion_refs,
              inputs: matrixEntry.input_contract_refs,
              outputs: matrixEntry.output_contract_refs,
            }}
          />
        ) : (
          <MissingProjectionCard
            key={matrixEntry.logical_work_item_id}
            logicalId={matrixEntry.logical_work_item_id}
          />
        );
      })}
      <ProviderRenderingStatus role="Reviewer" />
      <ValidationSummary validation={validation} />
    </section>
  );
}

function ReviewerWorkItemCard({
  logicalId,
  projection,
  refs,
}: {
  logicalId: string;
  projection: ReviewerWorkItemProjection;
  refs: { criteria: string[]; inputs: string[]; outputs: string[] };
}) {
  return (
    <article className="rounded-md border border-[var(--aria-line)] bg-white p-4">
      <h4 className="font-mono text-sm font-semibold text-[var(--aria-ink)]">{logicalId}</h4>
      <p className="mt-1 break-words text-xs text-[var(--aria-ink-muted)]">
        Criteria: {refs.criteria.join(", ") || "--"} · Inputs: {refs.inputs.join(", ") || "--"} · Outputs: {refs.outputs.join(", ") || "--"}
      </p>
      <div className="mt-3 space-y-2">
        {projection.requirement_matrix.map((check) => (
          <div
            key={check.criterion_id}
            className="grid gap-2 rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 text-xs md:grid-cols-[minmax(7rem,0.6fr)_minmax(10rem,1fr)_minmax(9rem,0.8fr)]"
          >
            <span className="font-mono font-semibold text-[var(--aria-ink)]">
              {check.criterion_id}
            </span>
            <span className="break-words text-[var(--aria-ink-muted)]">
              {check.requirement_refs.join(", ") || "无 requirement ref"}
            </span>
            <span className="break-all font-mono text-[var(--aria-ink)]">
              {check.failure_route}
            </span>
          </div>
        ))}
      </div>
      <details className="mt-3 rounded border border-[var(--aria-line)] p-3">
        <summary className="cursor-pointer text-xs font-semibold text-[var(--aria-ink)]">
          完整 reviewer projection
        </summary>
        <pre className="mt-3 max-w-full overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs leading-5 text-[var(--aria-ink-muted)]">
          {JSON.stringify(projection, null, 2)}
        </pre>
      </details>
    </article>
  );
}

function HistoryView({ history }: { history: WorkItemRevisionHistoryDto | null }) {
  const entries = [...(history?.entries ?? [])].sort((left, right) =>
    left.created_at.localeCompare(right.created_at),
  );
  if (entries.length === 0) {
    return (
      <p className="rounded-md border border-[var(--aria-line)] bg-white p-4 text-sm text-[var(--aria-ink-muted)]">
        暂无 revision history artifact。
      </p>
    );
  }
  return (
    <ol className="space-y-3" aria-label="Work Item revision history">
      {entries.map((entry) => (
        <li
          key={`${entry.kind}-${entry.id}`}
          className="rounded-md border border-[var(--aria-line)] bg-white p-4"
        >
          <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
            <div>
              <div className="font-mono text-xs font-semibold text-[var(--aria-ink)]">
                {entry.id}
              </div>
              <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
                {entry.kind} · {entry.logical_work_item_id}
              </div>
            </div>
            <time className="text-xs text-[var(--aria-ink-muted)]" dateTime={entry.created_at}>
              {entry.created_at}
            </time>
          </div>
          <p className="mt-2 text-sm leading-6 text-[var(--aria-ink)]">{entry.summary}</p>
          {entry.related_revision_id ? (
            <p className="mt-2 break-all font-mono text-xs text-[var(--aria-ink-muted)]">
              related: {entry.related_revision_id}
            </p>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

function ValidationSummary({
  validation,
}: {
  validation: ProjectionValidationReport | null;
}) {
  const findings = validation?.findings ?? [];
  return (
    <section className="rounded-md border border-[var(--aria-line)] bg-white p-4">
      <h4 className="text-sm font-semibold text-[var(--aria-ink)]">Projection validation</h4>
      {findings.length === 0 ? (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">未发现 projection validation 问题。</p>
      ) : (
        <ul className="mt-2 space-y-2 text-xs text-[var(--aria-ink)]">
          {findings.map((finding) => (
            <li key={`${finding.code}-${finding.contract_ref ?? "none"}`}>
              <span className="font-mono font-semibold">{finding.code}</span> {finding.message}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function CountCard({ label, count }: { label: string; count: number }) {
  return (
    <div className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
      <div className="text-xs text-[var(--aria-ink-muted)]">{label}</div>
      <div className="mt-1 font-mono text-sm font-semibold text-[var(--aria-ink)]">{count}</div>
    </div>
  );
}

function MissingProjectionCard({ logicalId }: { logicalId: string }) {
  return (
    <div className="rounded-md border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900">
      {logicalId} 缺少对应的 Work Item projection artifact。
    </div>
  );
}
