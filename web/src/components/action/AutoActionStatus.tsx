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
    <section className="border-t border-line bg-white px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-semibold">{currentAction}</div>
          <div className="mt-1 text-xs text-slate-500">
            {events
              .slice(-3)
              .map((event) => event.event_type)
              .join(" · ")}
          </div>
        </div>
        <button type="button" className="rounded-md border border-line px-3 py-2 text-sm" onClick={onStop}>
          <Square className="mr-1 inline h-4 w-4" />
          停止
        </button>
      </div>
    </section>
  );
}
