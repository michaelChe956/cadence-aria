import { GitBranch } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function StageChangeEntry({ entry }: { entry: ChatEntry }) {
  return (
    <ChatEntryContainer role="system" title="阶段变更">
      <div className="flex items-center gap-2 text-sm text-[var(--aria-ink)]">
        <GitBranch className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="flex min-w-0 items-center gap-2">
          <span className="h-px w-8 bg-[var(--aria-line)]" aria-hidden="true" />
          <span className="truncate font-medium">{entry.content}</span>
          <span className="h-px flex-1 bg-[var(--aria-line)]" aria-hidden="true" />
        </div>
      </div>
    </ChatEntryContainer>
  );
}
