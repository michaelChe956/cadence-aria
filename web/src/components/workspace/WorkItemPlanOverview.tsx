import type {
  HumanGroupProjection,
  HumanPresentationRevision,
  SaveHumanPresentationRevisionMessage,
  WorkItemProjectionBundle,
} from "../../api/types";
import type { HumanPresentationSaveState } from "../../state/workspace-ws-store-types";
import { HumanPresentationEditor } from "./HumanPresentationEditor";

export function WorkItemPlanOverview({
  projection,
  presentation,
  planProjectionBundleId = null,
  workItemProjections = [],
  presentations = {},
  presentationSaveStates = {},
  editable = false,
  onSavePresentation = () => undefined,
}: {
  projection: HumanGroupProjection;
  presentation: HumanPresentationRevision | null;
  planProjectionBundleId?: string | null;
  workItemProjections?: WorkItemProjectionBundle[];
  presentations?: Record<string, HumanPresentationRevision>;
  presentationSaveStates?: Record<string, HumanPresentationSaveState>;
  editable?: boolean;
  onSavePresentation?: (message: SaveHumanPresentationRevisionMessage) => void;
}) {
  const planPresentation =
    presentation ??
    (planProjectionBundleId ? presentations[planProjectionBundleId] ?? null : null);
  const workItemProjectionByLogicalId = new Map(
    workItemProjections.map((bundle) => [
      bundle.human_projection.logical_work_item_id,
      bundle,
    ]),
  );
  return (
    <section aria-labelledby="work-item-plan-overview-title" className="space-y-4">
      <header className="rounded-md border border-[var(--aria-line)] bg-white p-4">
        <p className="text-xs font-semibold uppercase tracking-wide text-[var(--aria-ink-muted)]">
          Human projection
        </p>
        <h3
          id="work-item-plan-overview-title"
          className="mt-1 text-base font-semibold text-[var(--aria-ink)]"
        >
          {projection.goal}
        </h3>
        <p className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]">
          {planPresentation?.human_summary ?? projection.split_reason}
        </p>
        {planPresentation?.why_split ? (
          <p className="mt-2 text-xs leading-5 text-[var(--aria-ink-muted)]">
            为什么这样拆分：{planPresentation.why_split}
          </p>
        ) : null}
      </header>

      {editable && planProjectionBundleId ? (
        <HumanPresentationEditor
          base={{
            scope: "plan",
            source_projection_bundle_id: planProjectionBundleId,
            human_summary: projection.split_reason,
            why_split: projection.split_reason,
            dependency_explanation: projection.contract_flow.map(
              (edge) => `${edge.from} → ${edge.to}: ${edge.contract_id}`,
            ),
            risk_explanation: projection.risks,
            source_refs: projection.source_refs,
            presentation: planPresentation,
          }}
          onSave={onSavePresentation}
          saving={presentationSaveStates[planProjectionBundleId]?.saving ?? false}
          error={presentationSaveStates[planProjectionBundleId]?.error ?? null}
        />
      ) : null}

      <div className="grid gap-3 lg:grid-cols-2">
        {projection.work_items.map((workItem) => {
          const bundle = workItemProjectionByLogicalId.get(
            workItem.logical_work_item_id,
          );
          const workItemPresentation = bundle
            ? presentations[bundle.id] ?? null
            : null;
          return (
            <article
              key={workItem.logical_work_item_id}
              className="min-w-0 rounded-md border border-[var(--aria-line)] bg-white p-4"
            >
              <h4 className="break-words text-sm font-semibold text-[var(--aria-ink)]">
                {workItem.logical_work_item_id} {workItem.title}
              </h4>
              <p className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]">
                {workItemPresentation?.human_summary ?? workItem.goal}
              </p>
              <SummaryList
                label="依赖"
                values={
                  workItemPresentation?.dependency_explanation ??
                  workItem.depends_on
                }
              />
              <SummaryList label="提供" values={workItem.provides} />
              <SummaryList label="负责范围" values={workItem.scope_summary.owned_scopes} />
              <SummaryList label="禁止范围" values={workItem.scope_summary.forbidden_scopes} />
              <SummaryList
                label="风险说明"
                values={workItemPresentation?.risk_explanation ?? []}
              />
              <SummaryList
                label="来源引用"
                values={workItemPresentation?.source_refs ?? []}
              />
              {editable && bundle ? (
                <HumanPresentationEditor
                  base={{
                    scope: "work_item",
                    source_projection_bundle_id: bundle.id,
                    human_summary: bundle.human_projection.goal,
                    why_split: null,
                    dependency_explanation: bundle.human_projection.dependencies,
                    risk_explanation: [],
                    source_refs: bundle.human_projection.source_refs,
                    presentation: workItemPresentation,
                  }}
                  onSave={onSavePresentation}
                  saving={presentationSaveStates[bundle.id]?.saving ?? false}
                  error={presentationSaveStates[bundle.id]?.error ?? null}
                />
              ) : null}
            </article>
          );
        })}
      </div>

      <div className="grid gap-3 md:grid-cols-3">
        <SummaryCard
          title="依赖说明"
          values={planPresentation?.dependency_explanation ?? []}
          empty="暂无补充依赖说明"
        />
        <SummaryCard
          title="风险"
          values={planPresentation?.risk_explanation ?? projection.risks}
          empty="暂无已知风险"
        />
        <SummaryCard
          title="来源"
          values={planPresentation?.source_refs ?? projection.source_refs}
          empty="暂无来源引用"
          mono
        />
      </div>
    </section>
  );
}

function SummaryList({ label, values }: { label: string; values: string[] }) {
  if (values.length === 0) {
    return null;
  }
  return (
    <div className="mt-3">
      <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">{label}</div>
      <ul className="mt-1 space-y-1 text-xs leading-5 text-[var(--aria-ink)]">
        {values.map((value) => (
          <li key={value} className="break-words">
            {value}
          </li>
        ))}
      </ul>
    </div>
  );
}

function SummaryCard({
  title,
  values,
  empty,
  mono = false,
}: {
  title: string;
  values: string[];
  empty: string;
  mono?: boolean;
}) {
  return (
    <section className="rounded-md border border-[var(--aria-line)] bg-white p-4">
      <h4 className="text-sm font-semibold text-[var(--aria-ink)]">{title}</h4>
      {values.length > 0 ? (
        <ul
          className={`mt-2 space-y-1 text-xs leading-5 text-[var(--aria-ink-muted)] ${
            mono ? "font-mono" : ""
          }`}
        >
          {values.map((value) => (
            <li key={value} className="break-all">
              {value}
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">{empty}</p>
      )}
    </section>
  );
}
