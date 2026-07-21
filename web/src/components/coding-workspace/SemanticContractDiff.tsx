import type {
  ContractImpactReport,
  PlanAmendmentManifest,
  PlanProjectionBundle,
} from "../../api/types";
import { classifyPlanRepairImpact } from "./plan-repair-impact-classification";

export type SemanticContractDiffView = "contract" | "coder" | "reviewer";

export function SemanticContractDiff({
  amendment,
  projection,
  impact,
  view,
}: {
  amendment: PlanAmendmentManifest | null;
  projection: PlanProjectionBundle | null;
  impact: ContractImpactReport | null;
  view: SemanticContractDiffView;
}) {
  if (!amendment) {
    return <EmptySemanticDiff message="等待 Plan Amendment Manifest。" />;
  }
  const items = semanticImpactItems(amendment, projection, impact);

  if (view === "coder") {
    const scopes = projection?.coder_group_context.group_write_scopes ?? {};
    return (
      <section className="space-y-3" aria-labelledby="coder-semantic-diff-title">
        <DiffHeader
          id="coder-semantic-diff-title"
          title="Coder Projection 影响"
          description="使用已发布的 Coder group DTO 展示修订后的执行顺序与写入边界。"
        />
        {items.filter((item) => item.coderOrder !== null).map((item) => {
          const delta = item.delta;
          const writePolicy = scopes[item.logicalId];
          return (
            <article
              key={item.logicalId}
              data-testid="semantic-coder-work-item"
              data-logical-id={item.logicalId}
              className="rounded-md border border-[var(--aria-line)] bg-white p-4"
            >
              <h4 className="font-mono text-sm font-semibold text-[var(--aria-ink)]">
                {String((item.coderOrder ?? 0) + 1).padStart(2, "0")} · {item.logicalId}
              </h4>
              <SemanticChangeList
                changes={[
                  ...item.impactLabels,
                  ...(delta?.added_capabilities ?? []).map(
                    (value) => `新增能力 ${value}`,
                  ),
                  ...(delta?.changed_capabilities ?? []).map(
                    (value) => `调整能力 ${value}`,
                  ),
                  ...(delta?.acceptance_changed ? ["Acceptance criteria 已变更"] : []),
                  ...(delta?.verification_changed ? ["Verification checks 已变更"] : []),
                  ...(writePolicy?.exclusive_scopes ?? []),
                  ...(writePolicy?.forbidden_scopes ?? []).map(
                    (scope) => `禁止写入 ${scope}`,
                  ),
                ]}
              />
            </article>
          );
        })}
      </section>
    );
  }

  if (view === "reviewer") {
    const matrix = projection?.reviewer_group_matrix.work_items ?? [];
    return (
      <section className="space-y-3" aria-labelledby="reviewer-semantic-diff-title">
        <DiffHeader
          id="reviewer-semantic-diff-title"
          title="Reviewer Projection 影响"
          description="使用已发布的 Reviewer matrix DTO 展示需要重新验证的标准与契约。"
        />
        {items.map((item) => {
          const delta = item.delta;
          const workItem = matrix.find(
            (entry) => entry.logical_work_item_id === item.logicalId,
          );
          return (
            <article
              key={item.logicalId}
              className="rounded-md border border-[var(--aria-line)] bg-white p-4"
            >
              <h4 className="font-mono text-sm font-semibold text-[var(--aria-ink)]">
                {item.logicalId}
              </h4>
              <SemanticChangeList
                changes={[
                  ...item.impactLabels,
                  ...(workItem?.criterion_refs ?? []),
                  ...(workItem?.input_contract_refs ?? []).map(
                    (contract) => `输入契约 ${contract}`,
                  ),
                  ...(workItem?.output_contract_refs ?? []).map(
                    (contract) => `输出契约 ${contract}`,
                  ),
                  ...(delta?.verification_changed ? ["重新验证证据规则"] : []),
                ]}
              />
            </article>
          );
        })}
      </section>
    );
  }

  return (
    <section className="space-y-3" aria-labelledby="contract-semantic-diff-title">
      <DiffHeader
        id="contract-semantic-diff-title"
        title="Semantic Contract Diff"
        description="按 Contract 与 Capability 展示语义变化，不使用逐行文本 diff。"
      />
      {amendment.contract_deltas.length === 0 ? (
        <EmptySemanticDiff message="本次修订没有 Contract Delta。" />
      ) : (
        items.filter((item) => item.delta !== null).map((item) => {
          const delta = item.delta!;
          return (
          <article
            key={`${delta.logical_work_item_id}-${delta.next_revision_id}`}
            className="rounded-md border border-[var(--aria-line)] bg-white p-4"
          >
            <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
              <div>
                <h4 className="font-mono text-sm font-semibold text-[var(--aria-ink)]">
                  {delta.logical_work_item_id}
                </h4>
                <p className="mt-1 break-all text-xs text-[var(--aria-ink-muted)]">
                  {delta.previous_revision_id} → {delta.next_revision_id}
                </p>
              </div>
              <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-[11px] text-[var(--aria-ink-muted)]">
                {delta.kind}
              </span>
            </div>
            <div className="mt-3 grid gap-3 lg:grid-cols-2">
              <SemanticChangeList
                changes={[
                  ...delta.added_contracts.map((value) => `新增 Contract ${value}`),
                  ...delta.added_capabilities.map((value) => `新增 ${value}`),
                  ...delta.changed_capabilities.map((value) => `变更 ${value}`),
                  ...delta.added_capability_associations.map(
                    (item) => `${item.contract_id} → ${item.capability}`,
                  ),
                ]}
                empty="没有新增或变更"
                tone="added"
              />
              <SemanticChangeList
                changes={[
                  ...delta.removed_contracts.map((value) => `移除 Contract ${value}`),
                  ...delta.removed_capabilities.map((value) => `移除 ${value}`),
                  ...delta.removed_capability_associations.map(
                    (item) => `${item.contract_id} → ${item.capability}`,
                  ),
                ]}
                empty="没有移除"
                tone="removed"
              />
            </div>
          </article>
          );
        })
      )}
    </section>
  );
}

function semanticImpactItems(
  amendment: PlanAmendmentManifest,
  projection: PlanProjectionBundle | null,
  impact: ContractImpactReport | null,
) {
  const affected = new Set(
    impact
      ? classifyPlanRepairImpact(amendment, impact).actualAffected
      : [
          ...Object.keys(amendment.revised_work_items),
          ...amendment.contract_deltas.map((delta) => delta.logical_work_item_id),
          ...amendment.stale_units,
          ...amendment.revalidation_required_units,
          ...Object.keys(amendment.replacement_units),
          ...Object.values(amendment.replacement_units).flat(),
          amendment.resume_target.logical_work_item_id,
        ],
  );
  const ordered = projection?.coder_group_context.ordered_logical_work_item_ids ?? [];
  const logicalIds = [
    ...ordered.filter((logicalId) => affected.has(logicalId)),
    ...[...affected].filter((logicalId) => !ordered.includes(logicalId)),
  ];
  return logicalIds.map((logicalId) => ({
    logicalId,
    coderOrder: ordered.includes(logicalId) ? ordered.indexOf(logicalId) : null,
    delta:
      amendment.contract_deltas.find(
        (delta) => delta.logical_work_item_id === logicalId,
      ) ?? null,
    impactLabels: impactLabelsFor(logicalId, amendment, impact),
  }));
}

function impactLabelsFor(
  logicalId: string,
  amendment: PlanAmendmentManifest,
  impact: ContractImpactReport | null,
) {
  const labels: string[] = [];
  if (
    amendment.stale_units.includes(logicalId) ||
    impact?.direct_stale.includes(logicalId)
  ) {
    labels.push("影响：重新执行");
  }
  if (
    amendment.revalidation_required_units.includes(logicalId) ||
    impact?.direct_revalidation.includes(logicalId)
  ) {
    labels.push("影响：重新验证");
  }
  if (impact?.conditional_downstream.includes(logicalId)) {
    labels.push("影响：条件性下游");
  }
  const replacements = amendment.replacement_units[logicalId] ?? [];
  if (replacements.length > 0) {
    labels.push(`替换单元：${replacements.join(", ")}`);
  }
  return labels;
}

function DiffHeader({
  id,
  title,
  description,
}: {
  id: string;
  title: string;
  description: string;
}) {
  return (
    <header className="rounded-md border border-[var(--aria-line)] bg-white p-4">
      <h3 id={id} className="text-sm font-semibold text-[var(--aria-ink)]">
        {title}
      </h3>
      <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
        {description}
      </p>
    </header>
  );
}

function SemanticChangeList({
  changes,
  empty = "暂无语义变化",
  tone = "neutral",
}: {
  changes: string[];
  empty?: string;
  tone?: "neutral" | "added" | "removed";
}) {
  const toneClass =
    tone === "added"
      ? "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-ink)]"
      : tone === "removed"
        ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-ink)]"
        : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink)]";
  return changes.length > 0 ? (
    <ul className="space-y-2 text-xs">
      {changes.map((change) => (
        <li key={change} className={`rounded border px-3 py-2 ${toneClass}`}>
          {change}
        </li>
      ))}
    </ul>
  ) : (
    <p className="rounded border border-dashed border-[var(--aria-line)] px-3 py-2 text-xs text-[var(--aria-ink-muted)]">
      {empty}
    </p>
  );
}

function EmptySemanticDiff({ message }: { message: string }) {
  return (
    <p className="rounded-md border border-[var(--aria-line)] bg-white p-4 text-sm text-[var(--aria-ink-muted)]">
      {message}
    </p>
  );
}
