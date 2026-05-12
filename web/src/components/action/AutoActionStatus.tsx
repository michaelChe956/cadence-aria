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
    <section className="rounded-xl border border-cyan-300/15 bg-white/[0.03] px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-slate-100">{currentAction}</div>
          <div className="mt-1 text-xs text-slate-500">
            {events
              .slice(-3)
              .map((event) => event.event_type)
              .join(" · ")}
          </div>
        </div>
        <button type="button" className="rounded-md border border-white/10 px-3 py-2 text-sm text-slate-200" onClick={onStop}>
          <Square className="mr-1 inline h-4 w-4" />
          停止
        </button>
      </div>
    </section>
  );
}
