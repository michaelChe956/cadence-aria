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
    <section className="rounded-lg border-2 border-emerald-200 bg-emerald-50 px-4 py-3 shadow-[0_8px_0_rgba(16,185,129,0.14)]">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-bold text-emerald-950">{currentAction}</div>
          <div className="mt-1 text-xs font-semibold text-emerald-700">
            {events
              .slice(-3)
              .map((event) => event.event_type)
              .join(" · ")}
          </div>
        </div>
        <button
          type="button"
          className="inline-flex items-center rounded-lg border-2 border-rose-300 bg-white px-3 py-2 text-sm font-bold text-rose-800 shadow-[0_4px_0_rgba(251,113,133,0.20)] transition-colors hover:bg-rose-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-rose-200"
          onClick={onStop}
        >
          <Square className="mr-1 inline h-4 w-4" />
          停止
        </button>
      </div>
    </section>
  );
}
