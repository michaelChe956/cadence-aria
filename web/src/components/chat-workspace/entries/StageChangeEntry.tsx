import { GitBranch } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { workspaceStageLabel } from "../../../state/workspace-stage-labels";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function StageChangeEntry({ entry }: { entry: ChatEntry }) {
  const stage = stageFromEntry(entry);
  const label = stage ? workspaceStageLabel(stage) : entry.content;

  return (
    <ChatEntryContainer role="system" title="阶段变更">
      <div className="flex items-center gap-2 text-sm text-[var(--aria-ink)]">
        <GitBranch className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="flex min-w-0 items-center gap-2">
          <span className="h-px w-8 bg-[var(--aria-line)]" aria-hidden="true" />
          <span className="truncate font-medium">{label}</span>
          <span className="h-px flex-1 bg-[var(--aria-line)]" aria-hidden="true" />
        </div>
      </div>
    </ChatEntryContainer>
  );
}

function stageFromEntry(entry: ChatEntry) {
  const metadataStage = entry.metadata?.stage;
  if (typeof metadataStage === "string" && metadataStage.length > 0) {
    return metadataStage;
  }

  const match = entry.content.match(/->\s*([a-z0-9_]+)/i);
  return match?.[1] ?? null;
}
