import { GitMerge } from "lucide-react";
import type { CodingTimelineNode } from "../../api/types";

export function PlanRepairTimelineGroup({
  nodes,
  activeNodeId,
  selectedNodeId,
  onSelectNode,
}: {
  nodes: CodingTimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
}) {
  return (
    <section
      role="group"
      aria-label="Plan Repair Timeline"
      className="rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] p-2"
    >
      <header className="mb-2 flex items-center gap-2 px-1 text-xs font-semibold text-[var(--aria-primary)]">
        <GitMerge className="h-4 w-4 shrink-0" />
        <span>Plan Repair</span>
      </header>
      <div className="space-y-2 border-l-2 border-[var(--aria-primary)] pl-2">
        {nodes.map((node) => {
          const active = node.id === activeNodeId;
          const selected = node.id === selectedNodeId;
          return (
            <button
              key={node.id}
              type="button"
              onClick={() => onSelectNode(node.id)}
              aria-current={active ? "step" : undefined}
              className={[
                "block w-full rounded-md border bg-white px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
                active || selected
                  ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
                  : "border-[var(--aria-line)] hover:border-[var(--aria-primary)]",
              ].join(" ")}
            >
              <div className="flex min-w-0 items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                    {node.title}
                  </div>
                  {node.summary ? (
                    <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
                      {node.summary}
                    </p>
                  ) : null}
                </div>
                <span className="shrink-0 rounded bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[11px] text-[var(--aria-ink-muted)]">
                  {node.status}
                </span>
              </div>
            </button>
          );
        })}
      </div>
    </section>
  );
}
