import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ProviderStreamEntry({ entry }: { entry: ChatEntry }) {
  return (
    <ChatEntryContainer
      role={entry.role === "reviewer" ? "reviewer" : "author"}
      title={entryTitle(entry)}
    >
      <MarkdownContent content={entry.content} />
    </ChatEntryContainer>
  );
}

const PROVIDER_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  fake: "Fake",
};

function entryTitle(entry: ChatEntry) {
  const base = entry.role === "reviewer" ? "审核者" : "作者";
  const provider = metadataProvider(entry.metadata);
  return provider ? `${base} · ${providerLabel(provider)}` : base;
}

function metadataProvider(metadata: ChatEntry["metadata"]) {
  const provider = metadata?.provider ?? metadata?.agent;
  return typeof provider === "string" && provider.length > 0 ? provider : null;
}

function providerLabel(provider: string) {
  return PROVIDER_LABELS[provider] ?? provider;
}

function MarkdownContent({ content }: { content: string }) {
  const blocks = markdownBlocks(content);
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
          <div key={index} className="whitespace-pre-wrap break-words text-sm text-[var(--aria-ink)]">
            {block}
          </div>
        );
      })}
    </div>
  );
}

function markdownBlocks(content: string) {
  const normalized = normalizeProviderContent(content);
  const blocks: string[] = [];
  let current: string[] = [];

  function flushCurrent() {
    const block = current.join("\n").trim();
    if (block) {
      blocks.push(block);
    }
    current = [];
  }

  for (const line of normalized.split("\n")) {
    if (line.trim().length === 0) {
      flushCurrent();
      continue;
    }
    if (/^#{1,6}\s+/.test(line.trim())) {
      flushCurrent();
      blocks.push(line.trim());
      continue;
    }
    current.push(line);
  }
  flushCurrent();

  return blocks;
}

function normalizeProviderContent(content: string) {
  const normalized = content.replace(/\r\n?/g, "\n").replace(/\\n/g, "\n");
  return normalized
    .split("\n")
    .map((line) =>
      line.length > 80 ? line.replace(/([。！？.!?])\s+(?=\S)/g, "$1\n") : line,
    )
    .join("\n");
}

export { MarkdownContent };
