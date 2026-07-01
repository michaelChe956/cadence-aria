import { FileText, Wrench } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ExecutionEventEntry({ entry }: { entry: ChatEntry }) {
  const event = entry.metadata as Record<string, unknown> | undefined;
  const isProviderPrompt =
    entry.content_ref?.kind === "provider_prompt" || event?.title === "Provider Prompt";
  const EventIcon = isProviderPrompt ? FileText : Wrench;

  return (
    <ChatEntryContainer
      role="system"
      title={isProviderPrompt ? "Provider Prompt" : "执行事件"}
      className={isProviderPrompt ? "border-solid border-[var(--aria-line)] bg-white" : ""}
    >
      <div className="space-y-2">
        <div className="flex items-start gap-2 text-sm text-[var(--aria-ink)]">
          <EventIcon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <div className="min-w-0">
            <div className="font-medium">{entry.content}</div>
            {event?.command && typeof event.command === "string" ? (
              <div className="mt-1 rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
                {event.command}
              </div>
            ) : null}
            {event?.output && typeof event.output === "string" ? (
              <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink)]">
                {event.output}
              </pre>
            ) : null}
          </div>
        </div>
      </div>
    </ChatEntryContainer>
  );
}
