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
    <section className="border-t-2 border-indigo-100 bg-white/75 px-4 py-2 backdrop-blur md:px-6 lg:px-8">
      <div className="flex flex-wrap gap-2 text-xs">
        {grouped.map(({ group, items }) => (
          <span
            key={group}
            className={
              items.length > 0
                ? "rounded-lg border-2 border-rose-200 bg-rose-100 px-2 py-1 font-bold text-rose-900"
                : "rounded-lg border-2 border-indigo-100 bg-indigo-50 px-2 py-1 font-bold text-indigo-500"
            }
          >
            {group}: {items.length}
          </span>
        ))}
      </div>
    </section>
  );
}
