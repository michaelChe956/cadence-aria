import { Package } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ArtifactUpdateEntry({ entry }: { entry: ChatEntry }) {
  return (
    <ChatEntryContainer role="system" title="产物更新">
      <div className="flex items-center gap-2 text-sm text-[var(--aria-ink)]">
        <Package className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <span className="inline-flex items-center rounded-full border border-emerald-200 bg-emerald-50 px-2 py-0.5 text-xs font-semibold text-emerald-700">
          {entry.content}
        </span>
      </div>
    </ChatEntryContainer>
  );
}
