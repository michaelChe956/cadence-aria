import type { HumanGroupProjection } from "../../api/types";

export function WorkItemContractFlow({
  projection,
}: {
  projection: HumanGroupProjection;
}) {
  if (projection.contract_flow.length === 0) {
    return (
      <p className="rounded-md border border-[var(--aria-line)] bg-white p-4 text-sm text-[var(--aria-ink-muted)]">
        当前计划没有跨 Work Item 的依赖契约。
      </p>
    );
  }

  return (
    <section aria-label="Work Item contract flow" className="space-y-3">
      {projection.contract_flow.map((edge, index) => {
        const provided = new Set(edge.provided_capabilities);
        const matched = edge.required_capabilities.filter((capability) =>
          provided.has(capability),
        );
        return (
          <article
            key={`${edge.from}-${edge.to}-${edge.contract_id}-${index}`}
            className="rounded-md border border-[var(--aria-line)] bg-white p-4"
          >
            <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
              <h3 className="font-mono text-sm font-semibold text-[var(--aria-ink)]">
                {edge.from} → {edge.to}
              </h3>
              <span className="max-w-full break-all rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
                {edge.contract_id}
              </span>
            </div>
            <div className="mt-3 grid gap-3 lg:grid-cols-3">
              <CapabilityList
                title="Required"
                prefix="需要"
                capabilities={edge.required_capabilities}
              />
              <CapabilityList
                title="Provided intersection"
                prefix="已提供"
                capabilities={matched}
              />
              <CapabilityList
                title="Missing"
                prefix="缺少"
                capabilities={edge.missing_capabilities}
                missing
              />
            </div>
          </article>
        );
      })}
    </section>
  );
}

function CapabilityList({
  title,
  prefix,
  capabilities,
  missing = false,
}: {
  title: string;
  prefix: string;
  capabilities: string[];
  missing?: boolean;
}) {
  return (
    <section
      className={`min-w-0 rounded border p-3 ${
        missing && capabilities.length > 0
          ? "border-amber-300 bg-amber-50"
          : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
      }`}
    >
      <h4 className="text-xs font-semibold text-[var(--aria-ink-muted)]">{title}</h4>
      {capabilities.length > 0 ? (
        <ul className="mt-2 space-y-1 font-mono text-xs text-[var(--aria-ink)]">
          {capabilities.map((capability) => (
            <li key={capability} className="break-all">
              {prefix} {capability}
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">无</p>
      )}
    </section>
  );
}
