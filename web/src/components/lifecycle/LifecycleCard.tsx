import { GitBranch, Layers3, ListChecks, ScrollText } from "lucide-react";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";

export function LifecycleCard({
  card,
  selected,
  onSelect,
}: {
  card: LifecycleCardData;
  selected: boolean;
  onSelect: () => void;
}) {
  const Icon =
    card.kind === "issue"
      ? ListChecks
      : card.kind === "story_spec"
        ? ScrollText
        : card.kind === "design_spec"
          ? Layers3
          : GitBranch;

  return (
    <button
      type="button"
      aria-label={card.title}
      aria-pressed={selected}
      onClick={onSelect}
      className={
        selected
          ? "w-full rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] p-3 text-left ring-2 ring-[var(--aria-primary)]"
          : "w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-left transition-colors hover:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      }
    >
      <span className="flex min-w-0 items-start gap-2">
        <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
            {card.title}
          </span>
          <span className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            <span>{card.id}</span>
            <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
              {card.status}
            </span>
          </span>
        </span>
      </span>
    </button>
  );
}
