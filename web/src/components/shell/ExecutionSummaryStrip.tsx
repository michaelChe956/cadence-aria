type ExecutionSummaryStripProps = {
  activeTaskId: string | null;
  selectedNodeId: string | null;
  nodeCount: number;
  artifactCount: number;
  eventCount: number;
};

export function ExecutionSummaryStrip({
  activeTaskId,
  selectedNodeId,
  nodeCount,
  artifactCount,
  eventCount,
}: ExecutionSummaryStripProps) {
  return (
    <section
      role="region"
      aria-label="执行摘要"
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
    >
      <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
        <SummaryMetric label="Task" value={activeTaskId ?? "no task"} mono />
        <SummaryMetric label="Selected node" value={selectedNodeId ?? "none"} mono />
        <SummaryMetric label="Nodes" value={String(nodeCount)} />
        <SummaryMetric label="Artifacts" value={String(artifactCount)} />
        <SummaryMetric label="Events" value={String(eventCount)} />
      </div>
    </section>
  );
}

function SummaryMetric({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2">
      <div className="text-[10px] font-semibold uppercase text-[var(--aria-ink-muted)]">
        {label}
      </div>
      <div
        className={
          mono
            ? "mt-1 truncate font-mono text-xs font-semibold text-[var(--aria-ink)]"
            : "mt-1 text-sm font-semibold text-[var(--aria-ink)]"
        }
      >
        {value}
      </div>
    </div>
  );
}
