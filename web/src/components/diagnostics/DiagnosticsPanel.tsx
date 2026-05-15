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
  const unknown = diagnostics.filter(
    (item) =>
      !diagnosticGroups.some((group) => item.category === group || item.code === group),
  );
  const allGroups =
    unknown.length > 0 ? [...grouped, { group: "unknown", items: unknown }] : grouped;

  if (diagnostics.length === 0) {
    return null;
  }

  return (
    <section className="border-t border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-2 md:px-6 lg:px-8">
      <div className="flex flex-wrap gap-2 text-xs">
        {allGroups.map(({ group, items }) => (
          <span
            key={group}
            className={
              items.length > 0
                ? "rounded-md border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-2 py-1 font-semibold text-[var(--aria-danger)]"
                : "rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1 font-semibold text-[var(--aria-ink-muted)]"
            }
          >
            {group}: {items.length}
          </span>
        ))}
      </div>
    </section>
  );
}
