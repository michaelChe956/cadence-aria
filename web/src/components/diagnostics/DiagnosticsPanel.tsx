const diagnosticGroups = [
  "provider_error",
  "gate_blocked",
  "validation_failed",
  "checkpoint_unsafe",
  "web_runtime_error",
];

export function DiagnosticsPanel({ diagnostics }: { diagnostics: Array<Record<string, unknown>> }) {
  const grouped = diagnosticGroups.map((group) => ({
    group,
    items: diagnostics.filter((item) => item.category === group || item.code === group),
  }));

  return (
    <section className="border-t border-line bg-white px-4 py-2">
      <div className="flex flex-wrap gap-3 text-xs">
        {grouped.map(({ group, items }) => (
          <span key={group} className={items.length > 0 ? "text-danger" : "text-slate-500"}>
            {group}: {items.length}
          </span>
        ))}
      </div>
    </section>
  );
}
