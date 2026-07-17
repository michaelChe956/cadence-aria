import type {
  HumanGroupProjection,
  HumanPresentationRevision,
} from "../../api/types";

export function WorkItemPlanOverview({
  projection,
  presentation,
}: {
  projection: HumanGroupProjection;
  presentation: HumanPresentationRevision | null;
}) {
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
          {presentation?.human_summary ?? projection.split_reason}
        </p>
      </header>

      <div className="grid gap-3 lg:grid-cols-2">
        {projection.work_items.map((workItem) => (
          <article
            key={workItem.logical_work_item_id}
            className="min-w-0 rounded-md border border-[var(--aria-line)] bg-white p-4"
          >
            <h4 className="break-words text-sm font-semibold text-[var(--aria-ink)]">
              {workItem.logical_work_item_id} {workItem.title}
            </h4>
            <p className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]">
              {workItem.goal}
            </p>
            <SummaryList label="依赖" values={workItem.depends_on} />
            <SummaryList label="提供" values={workItem.provides} />
            <SummaryList label="负责范围" values={workItem.scope_summary.owned_scopes} />
            <SummaryList label="禁止范围" values={workItem.scope_summary.forbidden_scopes} />
          </article>
        ))}
      </div>

      <div className="grid gap-3 md:grid-cols-2">
        <SummaryCard title="风险" values={projection.risks} empty="暂无已知风险" />
        <SummaryCard
          title="来源"
          values={projection.source_refs}
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
