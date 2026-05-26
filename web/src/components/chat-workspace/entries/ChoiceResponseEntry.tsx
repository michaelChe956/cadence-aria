import { Check } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";

export function ChoiceResponseEntry({ entry }: { entry: ChatEntry }) {
  return (
    <div className="flex justify-start">
      <span className="inline-flex items-center gap-1 rounded-full border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
        <Check className="h-3.5 w-3.5" />
        {entry.content}
      </span>
    </div>
  );
}
