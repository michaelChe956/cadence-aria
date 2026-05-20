import {
  GitBranch,
  Layers3,
  ListChecks,
  ScrollText,
  Trash2,
} from "lucide-react";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";

export function LifecycleCard({
  card,
  selected,
  onSelect,
  onDeleteIssue,
}: {
  card: LifecycleCardData;
  selected: boolean;
  onSelect: () => void;
  onDeleteIssue?: () => void;
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
    <div
      className={
        selected
          ? "flex w-full items-start gap-2 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] p-3 text-left ring-2 ring-[var(--aria-primary)]"
          : "flex w-full items-start gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-left transition-colors hover:bg-white focus-within:ring-2 focus-within:ring-[var(--aria-primary)]"
      }
    >
      <button
        type="button"
        aria-label={card.title}
        aria-pressed={selected}
        onClick={onSelect}
        className="min-w-0 flex-1 text-left focus-visible:outline-none"
      >
        <span className="flex min-w-0 items-start gap-2">
          <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
              {card.title}
            </span>
            {card.preview ? (
              <span className="mt-1 line-clamp-2 block whitespace-pre-wrap break-words text-xs leading-5 text-[var(--aria-ink-muted)]">
                {card.preview}
              </span>
            ) : null}
            <span className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
              <span>{card.id}</span>
              <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                {card.status}
              </span>
              {card.version ? (
                <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                  v{card.version}
                </span>
              ) : null}
            </span>
          </span>
        </span>
      </button>
      {onDeleteIssue ? (
        <button
          type="button"
          aria-label={`删除 Issue ${card.title}`}
          onClick={onDeleteIssue}
          className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)]"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      ) : null}
    </div>
  );
}
