import type {
  ContractImpactReport,
  PlanAmendmentManifest,
  PlanProjectionBundle,
} from "../../api/types";

export function ImpactPreview({
  amendment,
  impact,
  projection,
}: {
  amendment: PlanAmendmentManifest | null;
  impact: ContractImpactReport | null;
  projection: PlanProjectionBundle | null;
}) {
  if (!amendment || !impact) {
    return (
      <p className="rounded-md border border-[var(--aria-line)] bg-white p-4 text-sm text-[var(--aria-ink-muted)]">
        等待 Impact Report。
      </p>
    );
  }

  const titles = new Map(
    (projection?.human_group_projection.work_items ?? []).map((item) => [
      item.logical_work_item_id,
      item.title,
    ]),
  );
  const reexecute = new Set([
    ...impact.direct_stale,
    ...(amendment.resume_target.mode === "reexecute"
      ? [amendment.resume_target.logical_work_item_id]
      : []),
  ]);
  const revalidate = new Set([
    ...impact.direct_revalidation,
    ...(amendment.resume_target.mode === "revalidate"
      ? [amendment.resume_target.logical_work_item_id]
      : []),
  ]);

  return (
    <section className="space-y-4" aria-labelledby="impact-preview-title">
      <header className="rounded-md border border-[var(--aria-line)] bg-white p-4">
        <h3 id="impact-preview-title" className="text-sm font-semibold text-[var(--aria-ink)]">
          Impact Preview
        </h3>
        <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
          恢复执行前预览重新执行、重新验证与不受影响的 Work Item。
        </p>
      </header>
      <div className="grid gap-3 lg:grid-cols-3">
        <ImpactColumn
          title="重新执行"
          items={[...reexecute]}
          action="重新执行"
          titles={titles}
          tone="danger"
        />
        <ImpactColumn
          title="重新验证"
          items={[...revalidate]}
          action="重新验证"
          titles={titles}
          tone="warning"
        />
        <ImpactColumn
          title="不受影响"
          items={impact.unaffected}
          action="不受影响"
          titles={titles}
          tone="safe"
        />
      </div>
      {impact.conditional_downstream.length > 0 ? (
        <section className="rounded-md border border-[var(--aria-line)] bg-white p-4">
          <h4 className="text-sm font-semibold text-[var(--aria-ink)]">条件性下游</h4>
          <ul className="mt-2 space-y-1 text-xs text-[var(--aria-ink-muted)]">
            {impact.conditional_downstream.map((logicalId) => (
              <li key={logicalId}>{logicalId}：条件性下游</li>
            ))}
          </ul>
        </section>
      ) : null}
      {impact.explanation_paths.length > 0 ? (
        <section className="rounded-md border border-[var(--aria-line)] bg-white p-4">
          <h4 className="text-sm font-semibold text-[var(--aria-ink)]">影响路径</h4>
          <ol className="mt-2 space-y-2 text-xs text-[var(--aria-ink-muted)]">
            {impact.explanation_paths.map((path) => (
              <li key={`${path.from}-${path.to}-${path.contract_id}`}>
                <span className="font-mono text-[var(--aria-ink)]">
                  {path.from} → {path.to} · {path.contract_id}
                </span>
                {path.capability_refs.length > 0
                  ? ` · ${path.capability_refs.join(", ")}`
                  : ""}
              </li>
            ))}
          </ol>
        </section>
      ) : null}
    </section>
  );
}

function ImpactColumn({
  title,
  items,
  action,
  titles,
  tone,
}: {
  title: string;
  items: string[];
  action: string;
  titles: Map<string, string>;
  tone: "danger" | "warning" | "safe";
}) {
  const toneClass =
    tone === "danger"
      ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)]"
      : tone === "warning"
        ? "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)]"
        : "border-[var(--aria-success)] bg-[var(--aria-success-soft)]";
  return (
    <section className={`rounded-md border p-4 ${toneClass}`}>
      <h4 className="text-sm font-semibold text-[var(--aria-ink)]">{title}</h4>
      {items.length > 0 ? (
        <ul className="mt-2 space-y-2 text-xs text-[var(--aria-ink)]">
          {items.map((logicalId) => (
            <li key={logicalId}>
              <div className="font-semibold">{logicalId}：{action}</div>
              {titles.get(logicalId) ? (
                <div className="mt-0.5 text-[var(--aria-ink-muted)]">
                  {titles.get(logicalId)}
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">无</p>
      )}
    </section>
  );
}
