import { useMemo, useState, type ReactNode } from "react";
import { lexer, type Token, type Tokens } from "marked";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ProviderStreamEntry({ entry }: { entry: ChatEntry }) {
  const content =
    entry.role === "reviewer" ? stripTrailingReviewJsonContract(entry.content) : entry.content;
  return (
    <ChatEntryContainer
      role={entry.role === "reviewer" ? "reviewer" : "author"}
      title={entryTitle(entry)}
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

const LARGE_MARKDOWN_COLLAPSE_CHARS = 80_000;
const LARGE_MARKDOWN_PREVIEW_CHARS = 8_000;

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
  const [expanded, setExpanded] = useState(false);
  const isLarge = content.length > LARGE_MARKDOWN_COLLAPSE_CHARS;
  const visibleContent =
    isLarge && !expanded ? content.slice(0, LARGE_MARKDOWN_PREVIEW_CHARS) : content;
  const normalizedContent = useMemo(
    () => normalizeProviderContent(visibleContent),
    [visibleContent],
  );
  const tokens = useMemo(
    () =>
      lexer(normalizedContent).filter(
        (token) => token.type !== "space" && token.type !== "def",
      ),
    [normalizedContent],
  );
  if (tokens.length === 0) {
    return <div className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]" />;
  }

  return (
    <div className="space-y-2 break-words text-sm text-[var(--aria-ink)]">
      {isLarge && !expanded ? (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          内容较长，当前显示前 {LARGE_MARKDOWN_PREVIEW_CHARS} 字符。完整内容仍可展开查看。
        </div>
      ) : null}
      {tokens.map((token, index) => renderBlockToken(token, `block-${index}`))}
      {isLarge ? (
        <button
          className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-1 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
          onClick={() => setExpanded((value) => !value)}
          type="button"
        >
          {expanded ? "收起全文" : "展开全文"}
        </button>
      ) : null}
    </div>
  );
}

function renderBlockToken(token: Token, key: string): ReactNode {
  switch (token.type) {
    case "heading":
      return renderHeading(token as Tokens.Heading, key);
    case "paragraph": {
      const paragraphToken = token as Tokens.Paragraph;
      return (
        <p key={key} className="whitespace-pre-wrap break-words">
          {renderInlineTokens(paragraphToken.tokens, paragraphToken.text, key)}
        </p>
      );
    }
    case "text": {
      const textToken = token as Tokens.Text;
      return (
        <p key={key} className="whitespace-pre-wrap break-words">
          {renderInlineTokens(textToken.tokens, textToken.text, key)}
        </p>
      );
    }
    case "list":
      return renderList(token as Tokens.List, key);
    case "code": {
      const codeToken = token as Tokens.Code;
      return (
        <pre
          key={key}
          className="overflow-x-auto rounded border border-[var(--aria-border)] bg-white/70 px-3 py-2 text-xs"
        >
          <code>{codeToken.text}</code>
        </pre>
      );
    }
    case "blockquote": {
      const blockquoteToken = token as Tokens.Blockquote;
      return (
        <blockquote
          key={key}
          className="space-y-2 border-l-2 border-[var(--aria-border)] pl-3 text-[var(--aria-ink-muted)]"
        >
          {blockquoteToken.tokens.map((child, index) =>
            renderBlockToken(child, `${key}-quote-${index}`),
          )}
        </blockquote>
      );
    }
    case "table":
      return renderTable(token as Tokens.Table, key);
    case "hr":
      return <hr key={key} className="border-[var(--aria-border)]" />;
    case "html": {
      const htmlToken = token as Tokens.HTML;
      return (
        <p key={key} className="whitespace-pre-wrap break-words">
          {htmlToken.raw || htmlToken.text}
        </p>
      );
    }
    case "space":
    case "def":
      return null;
    default:
      return (
        <p key={key} className="whitespace-pre-wrap break-words">
          {token.raw}
        </p>
      );
  }
}

function renderHeading(token: Tokens.Heading, key: string) {
  const content = renderInlineTokens(token.tokens, token.text, key);
  if (token.depth === 1) {
    return (
      <h1 key={key} className="text-lg font-semibold text-[var(--aria-ink)]">
        {content}
      </h1>
    );
  }
  if (token.depth === 2) {
    return (
      <h2 key={key} className="text-base font-semibold text-[var(--aria-ink)]">
        {content}
      </h2>
    );
  }
  return (
    <h3 key={key} className="text-sm font-semibold text-[var(--aria-ink)]">
      {content}
    </h3>
  );
}

function renderList(token: Tokens.List, key: string) {
  const listClass = token.ordered
    ? "list-decimal space-y-1 pl-5"
    : "list-disc space-y-1 pl-5";
  const items = token.items.map((item, index) => (
    <li key={`${key}-item-${index}`} className="pl-1">
      {item.task ? (
        <input
          checked={Boolean(item.checked)}
          className="mr-2 align-middle"
          disabled
          readOnly
          type="checkbox"
        />
      ) : null}
      {renderListItemContent(item, `${key}-item-${index}`)}
    </li>
  ));

  if (token.ordered) {
    return (
      <ol
        key={key}
        className={listClass}
        start={typeof token.start === "number" ? token.start : undefined}
      >
        {items}
      </ol>
    );
  }

  return (
    <ul key={key} className={listClass}>
      {items}
    </ul>
  );
}

function renderListItemContent(item: Tokens.ListItem, key: string): ReactNode {
  if (item.tokens.length === 1 && item.tokens[0]?.type === "text") {
    const textToken = item.tokens[0] as Tokens.Text;
    return renderInlineTokens(textToken.tokens, textToken.text, key);
  }

  return (
    <div className="space-y-1">
      {item.tokens.map((child, index) => renderBlockToken(child, `${key}-child-${index}`))}
    </div>
  );
}

function renderTable(token: Tokens.Table, key: string) {
  return (
    <div key={key} className="overflow-x-auto">
      <table className="min-w-full border-collapse text-left text-sm">
        <thead>
          <tr>
            {token.header.map((cell, index) => (
              <th
                key={`${key}-header-${index}`}
                className="border border-[var(--aria-border)] bg-white/70 px-2 py-1 font-semibold"
                style={{ textAlign: cell.align ?? "left" }}
              >
                {renderInlineTokens(cell.tokens, cell.text, `${key}-header-${index}`)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {token.rows.map((row, rowIndex) => (
            <tr key={`${key}-row-${rowIndex}`}>
              {row.map((cell, cellIndex) => (
                <td
                  key={`${key}-cell-${rowIndex}-${cellIndex}`}
                  className="border border-[var(--aria-border)] px-2 py-1"
                  style={{ textAlign: cell.align ?? "left" }}
                >
                  {renderInlineTokens(
                    cell.tokens,
                    cell.text,
                    `${key}-cell-${rowIndex}-${cellIndex}`,
                  )}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function renderInlineTokens(
  tokens: Token[] | undefined,
  fallback: string,
  keyPrefix: string,
): ReactNode {
  if (!tokens || tokens.length === 0) {
    return fallback;
  }

  return tokens.map((token, index) => renderInlineToken(token, `${keyPrefix}-inline-${index}`));
}

function renderInlineToken(token: Token, key: string): ReactNode {
  switch (token.type) {
    case "text":
    case "escape":
      return (token as Tokens.Text | Tokens.Escape).text;
    case "strong": {
      const strongToken = token as Tokens.Strong;
      return (
        <strong key={key}>
          {renderInlineTokens(strongToken.tokens, strongToken.text, key)}
        </strong>
      );
    }
    case "em": {
      const emToken = token as Tokens.Em;
      return <em key={key}>{renderInlineTokens(emToken.tokens, emToken.text, key)}</em>;
    }
    case "codespan": {
      const codespanToken = token as Tokens.Codespan;
      return (
        <code
          key={key}
          className="rounded border border-[var(--aria-border)] bg-white/70 px-1 py-0.5 text-[0.85em]"
        >
          {codespanToken.text}
        </code>
      );
    }
    case "br":
      return <br key={key} />;
    case "del": {
      const delToken = token as Tokens.Del;
      return <del key={key}>{renderInlineTokens(delToken.tokens, delToken.text, key)}</del>;
    }
    case "link":
      return renderLinkToken(token as Tokens.Link, key);
    case "image": {
      const imageToken = token as Tokens.Image;
      return imageToken.text || imageToken.href;
    }
    case "html": {
      const htmlToken = token as Tokens.HTML;
      return htmlToken.raw || htmlToken.text;
    }
    default:
      return token.raw;
  }
}

function renderLinkToken(token: Tokens.Link, key: string) {
  const href = safeMarkdownHref(token.href);
  const content = renderInlineTokens(token.tokens, token.text, key);
  if (!href) {
    return <span key={key}>{content}</span>;
  }

  return (
    <a
      key={key}
      className="font-medium text-blue-700 underline underline-offset-2"
      href={href}
      rel="noreferrer"
      target={isExternalHref(href) ? "_blank" : undefined}
      title={token.title ?? undefined}
    >
      {content}
    </a>
  );
}

function safeMarkdownHref(href: string) {
  const trimmed = href.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.startsWith("#") || trimmed.startsWith("/") || trimmed.startsWith("./")) {
    return trimmed;
  }
  if (trimmed.startsWith("../")) {
    return trimmed;
  }

  try {
    const url = new URL(trimmed, "https://aria.local");
    return ["http:", "https:", "mailto:", "tel:"].includes(url.protocol) ? trimmed : null;
  } catch {
    return null;
  }
}

function isExternalHref(href: string) {
  try {
    const url = new URL(href, window.location.href);
    return url.origin !== window.location.origin;
  } catch {
    return false;
  }
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

export { MarkdownContent };
