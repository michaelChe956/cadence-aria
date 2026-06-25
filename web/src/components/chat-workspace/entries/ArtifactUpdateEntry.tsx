import { Package } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ArtifactUpdateEntry({ entry }: { entry: ChatEntry }) {
  const versionLabel =
    typeof entry.metadata?.version_label === "string" ? entry.metadata.version_label : null;
  const objectTitle =
    typeof entry.metadata?.object_title === "string" ? entry.metadata.object_title : null;

  return (
    <ChatEntryContainer role="system" title="产物更新">
      <div className="flex items-start gap-2 text-sm text-[var(--aria-ink)]">
        <Package className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="min-w-0">
          <div className="break-words font-medium">{entry.content}</div>
          {objectTitle ? (
            <div className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
              {objectTitle}
            </div>
          ) : null}
          {versionLabel ? (
            <div className="mt-1 text-[11px] text-[var(--aria-ink-muted)]">{versionLabel}</div>
          ) : null}
        </div>
      </div>
    </ChatEntryContainer>
  );
}
