import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ProviderStreamEntry({ entry }: { entry: ChatEntry }) {
  return (
    <ChatEntryContainer
      role={entry.role === "reviewer" ? "reviewer" : "author"}
      title={entry.role === "reviewer" ? "审核者" : "作者"}
    >
      <MarkdownContent content={entry.content} />
    </ChatEntryContainer>
  );
}

function MarkdownContent({ content }: { content: string }) {
  const blocks = content.split(/\n{2,}/).filter((block) => block.trim().length > 0);
  if (blocks.length === 0) {
    return <div className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]" />;
  }

  return (
    <div className="space-y-2">
      {blocks.map((block, index) => {
        const trimmed = block.trim();
        if (trimmed.startsWith("### ")) {
          return (
            <h3 key={index} className="text-sm font-semibold text-[var(--aria-ink)]">
              {trimmed.slice(4)}
            </h3>
          );
        }
        if (trimmed.startsWith("## ")) {
          return (
            <h2 key={index} className="text-base font-semibold text-[var(--aria-ink)]">
              {trimmed.slice(3)}
            </h2>
          );
        }
        if (trimmed.startsWith("# ")) {
          return (
            <h1 key={index} className="text-lg font-semibold text-[var(--aria-ink)]">
              {trimmed.slice(2)}
            </h1>
          );
        }
        return (
          <div key={index} className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]">
            {block}
          </div>
        );
      })}
    </div>
  );
}
