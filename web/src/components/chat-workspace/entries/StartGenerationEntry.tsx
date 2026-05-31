import { Play } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function StartGenerationEntry({ entry }: { entry: ChatEntry }) {
  const snapshot = snapshotFromEntry(entry);

  return (
    <ChatEntryContainer role="system" title="生成事件">
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
          <Play className="h-4 w-4 text-[var(--aria-primary)]" />
          <span>{entry.content}</span>
        </div>
        {snapshot ? (
          <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--aria-ink-muted)]">
            <span>Author: {snapshot.author}</span>
            {snapshot.reviewer ? <span>Reviewer: {snapshot.reviewer} · {snapshot.review_rounds} 轮</span> : null}
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function snapshotFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const snapshot = metadata?.snapshot;
  if (!isRecord(snapshot)) {
    return null;
  }
  const author = stringField(snapshot, "author");
  if (!author) {
    return null;
  }
  const reviewer = stringField(snapshot, "reviewer");
  const reviewRounds = typeof snapshot.review_rounds === "number" ? snapshot.review_rounds : 0;
  return {
    author,
    reviewer,
    review_rounds: reviewRounds,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringField(value: Record<string, unknown>, key: string) {
  const field = value[key];
  return typeof field === "string" ? field : null;
}
