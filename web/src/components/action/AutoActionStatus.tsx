import { Square } from "lucide-react";
import type { WebEvent } from "../../api/types";

export function AutoActionStatus({
  currentAction,
  events,
  onStop,
}: {
  currentAction: string;
  events: WebEvent[];
  onStop: () => void;
}) {
  return (
    <section className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-3 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-[var(--aria-ink)]">{currentAction}</div>
          <div className="mt-1 break-words text-xs font-medium text-[var(--aria-ink-muted)]">
            {events
              .slice(-3)
              .map((event) => event.event_type)
              .join(" · ")}
          </div>
        </div>
        <button
          type="button"
          className="inline-flex h-9 shrink-0 items-center rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)]"
          onClick={onStop}
        >
          <Square className="mr-1 inline h-4 w-4" />
          停止
        </button>
      </div>
    </section>
  );
}
