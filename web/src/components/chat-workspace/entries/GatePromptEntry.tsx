import { Check, X } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function GatePromptEntry({
  entry,
  onDecision,
}: {
  entry: ChatEntry;
  onDecision?: (decision: "confirm" | "terminate") => void;
}) {
  const summary = summaryFromEntry(entry);

  return (
    <ChatEntryContainer
      role="system"
      title="人工确认"
      className="border-slate-200 bg-slate-50"
      testId="gate-prompt-entry"
    >
      <div className="space-y-3">
        <div className="text-sm text-[var(--aria-ink)]">{entry.content}</div>
        {summary ? <div className="text-xs text-[var(--aria-ink-muted)]">{summary}</div> : null}
        {onDecision ? (
          <div className="flex flex-wrap justify-end gap-2">
            <button
              type="button"
              onClick={() => onDecision("confirm")}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50"
            >
              <Check className="h-3.5 w-3.5" />
              确认
            </button>
            <button
              type="button"
              onClick={() => onDecision("terminate")}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50"
            >
              <X className="h-3.5 w-3.5" />
              终止
            </button>
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function summaryFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.summary === "string" ? metadata.summary : null;
}
