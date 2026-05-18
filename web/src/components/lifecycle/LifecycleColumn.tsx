import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { LifecycleCard } from "./LifecycleCard";

export function LifecycleColumn({
  title,
  ariaLabel,
  cards,
  selectedKey,
  onSelect,
  onOpenWorkspace,
  onDeleteIssue,
}: {
  title: string;
  ariaLabel: string;
  cards: LifecycleCardData[];
  selectedKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onOpenWorkspace?: (card: LifecycleCardData) => void;
  onDeleteIssue?: (issueId: string) => void;
}) {
  return (
    <section
      role="region"
      aria-label={ariaLabel}
      className="min-h-96 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">{title}</h2>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {cards.length}
        </span>
      </div>
      <ul className="space-y-2">
        {cards.map((card) => (
          <li key={`${card.kind}:${card.id}`}>
            <LifecycleCard
              card={card}
              selected={selectedKey === `${card.kind}:${card.id}`}
              onSelect={() => onSelect(card)}
              onOpenWorkspace={
                card.kind === "issue" || !onOpenWorkspace ? undefined : () => onOpenWorkspace(card)
              }
              onDeleteIssue={
                card.kind === "issue" && onDeleteIssue ? () => onDeleteIssue(card.id) : undefined
              }
            />
          </li>
        ))}
      </ul>
    </section>
  );
}
