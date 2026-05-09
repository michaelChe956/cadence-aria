import type { ReactNode } from "react";
import { useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";

function textFromNode(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }
  if (Array.isArray(node)) {
    return node.map(textFromNode).join("");
  }
  return "";
}

function slug(text: string) {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, "-")
    .replace(/^-|-$/g, "");
}

export function ArtifactContentRenderer({
  contentType,
  content,
}: {
  contentType: string;
  content: string;
}) {
  if (contentType === "markdown") {
    const headings = content
      .split("\n")
      .filter((line) => line.startsWith("#"))
      .map((line) => line.replace(/^#+\s*/, ""));
    return (
      <div className="grid grid-cols-[12rem_minmax(0,1fr)] gap-4 p-3">
        <nav aria-label="Markdown outline" className="text-sm">
          {headings.map((heading) => (
            <a key={heading} href={`#${slug(heading)}`} className="block py-1 text-slate-600">
              {heading}
            </a>
          ))}
        </nav>
        <ReactMarkdown
          components={{
            h1: ({ children }) => <h1 id={slug(textFromNode(children))}>{children}</h1>,
            h2: ({ children }) => <h2 id={slug(textFromNode(children))}>{children}</h2>,
            h3: ({ children }) => <h3 id={slug(textFromNode(children))}>{children}</h3>,
          }}
        >
          {content}
        </ReactMarkdown>
      </div>
    );
  }

  if (contentType === "json") {
    return <JsonContent content={content} />;
  }

  return <pre className="max-h-[34rem] overflow-auto p-3 text-xs leading-5">{content}</pre>;
}

function JsonContent({ content }: { content: string }) {
  const value = useMemo(() => {
    try {
      return JSON.parse(content) as Record<string, unknown>;
    } catch {
      return { value: content };
    }
  }, [content]);
  return (
    <div className="space-y-2 p-3 text-xs">
      {Object.entries(value).map(([key, item]) => (
        <JsonField key={key} name={key} value={item} />
      ))}
    </div>
  );
}

function JsonField({ name, value }: { name: string; value: unknown }) {
  const [open, setOpen] = useState(false);
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  const long = text.length > 120;
  return (
    <div>
      <strong>{name}</strong>
      {long && !open ? (
        <button
          type="button"
          aria-label={`展开 ${name}`}
          className="ml-2 text-signal"
          onClick={() => setOpen(true)}
        >
          展开
        </button>
      ) : null}
      <pre className="mt-1 rounded bg-slate-50 p-2">
        {long && !open ? `${text.length} chars hidden` : text}
      </pre>
    </div>
  );
}
