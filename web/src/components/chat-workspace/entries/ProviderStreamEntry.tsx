import type { ChatEntry } from "../../../state/chat-entries";
import type { UsageReportPayload } from "../../../api/types/workspace";
import { ChatEntryContainer } from "../ChatEntryContainer";
import { MarkdownContent } from "../../shared/MarkdownContent";

export function ProviderStreamEntry({ entry }: { entry: ChatEntry }) {
  const content = normalizeProviderStreamEntryContent(entry);
  const usage = entryUsage(entry);
  return (
    <ChatEntryContainer
      role={entry.role}
      title={entryTitle(entry)}
      titleSuffix={usage ? (
        <span
          data-testid="provider-stream-usage"
          className="ml-2 border-l border-gray-300 pl-2 text-[11px] font-normal text-amber-600"
        >
          {usage}
        </span>
      ) : null}
    >
      <MarkdownContent content={content} />
    </ChatEntryContainer>
  );
}

const PROVIDER_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  fake: "Fake",
};

function entryTitle(entry: ChatEntry) {
  const base = ROLE_LABELS[entry.role] ?? entry.role;
  const provider = metadataProvider(entry.metadata);
  return provider ? `${base} · ${providerLabel(provider)}` : base;
}

const ROLE_LABELS: Record<string, string> = {
  user: "用户",
  author: "作者",
  coder: "Coder",
  reviewer: "审核者",
  code_reviewer: "Code Reviewer",
  internal_reviewer: "Internal Reviewer",
  system: "系统",
};

export function entryUsage(entry: ChatEntry): string | null {
  const usage = entry.metadata?.usage as UsageReportPayload | undefined;
  if (!usage) {
    return null;
  }
  const segments: string[] = [];
  if (typeof usage.input_tokens === "number" && usage.input_tokens > 0) {
    segments.push(`输入 ${usage.input_tokens.toLocaleString("en-US")}`);
  }
  if (typeof usage.output_tokens === "number" && usage.output_tokens > 0) {
    segments.push(`输出 ${usage.output_tokens.toLocaleString("en-US")}`);
  }
  if (typeof usage.cache_read_tokens === "number" && usage.cache_read_tokens > 0) {
    segments.push(`缓存 ${usage.cache_read_tokens.toLocaleString("en-US")}`);
  }
  if (segments.length === 0) {
    return null;
  }
  return `Tokens ${segments.join(" · ")}`;
}

function metadataProvider(metadata: ChatEntry["metadata"]) {
  const provider = metadata?.provider ?? metadata?.agent;
  return typeof provider === "string" && provider.length > 0 ? provider : null;
}

function providerLabel(provider: string) {
  return PROVIDER_LABELS[provider] ?? provider;
}

function normalizeEntryContent(entry: ChatEntry) {
  return entry.content;
}

function stripTrailingReviewJsonContract(content: string) {
  const match = content.match(/\n?```json\s*([\s\S]*?)\s*```\s*$/u);
  if (!match || match.index === undefined) {
    return content;
  }

  try {
    const value = JSON.parse(match[1]);
    if (value && typeof value === "object" && "verdict" in value && "findings" in value) {
      return content.slice(0, match.index).trimEnd();
    }
  } catch {
    return content;
  }

  return content;
}

function normalizeProviderStreamEntryContent(entry: ChatEntry) {
  return entry.role === "reviewer" || entry.role === "code_reviewer"
    ? stripTrailingReviewJsonContract(entry.content)
    : normalizeEntryContent(entry);
}

export { MarkdownContent } from "../../shared/MarkdownContent";
export { normalizeProviderStreamEntryContent };
