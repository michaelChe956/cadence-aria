import { ChevronDown, ChevronRight, Wrench } from "lucide-react";
import { useState } from "react";
import type { ChatEntry } from "../../state/chat-entries";

export function InlineEventRow({ entry }: { entry: ChatEntry }) {
  const [expanded, setExpanded] = useState(false);
  const event = entry.metadata as Record<string, unknown> | undefined;
  const command = typeof event?.command === "string" ? event.command : null;
  const output = typeof event?.output === "string" ? event.output : null;
  const detail = typeof event?.detail === "string" ? event.detail : null;

  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]">
      <button
        type="button"
        onClick={() => setExpanded((current) => !current)}
        className="flex min-h-9 w-full min-w-0 items-center gap-2 px-2 text-left text-xs font-medium text-[var(--aria-ink)] hover:bg-white"
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-[var(--aria-ink-muted)]" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-[var(--aria-ink-muted)]" />
        )}
        <Wrench className="h-3.5 w-3.5 shrink-0 text-[var(--aria-primary)]" />
        <span className="min-w-0 truncate">{entry.content}</span>
      </button>
      {expanded ? (
        <div className="space-y-2 border-t border-[var(--aria-line)] px-2 py-2">
          {detail ? <div className="text-xs text-[var(--aria-ink-muted)]">{detail}</div> : null}
          {command ? (
            <div className="rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
              {command}
            </div>
          ) : null}
          {output ? (
            <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink)]">
              {output}
            </pre>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
