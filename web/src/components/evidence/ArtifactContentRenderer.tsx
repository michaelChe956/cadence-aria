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
      <div className="grid gap-4 p-3 lg:grid-cols-[12rem_minmax(0,1fr)]">
        <nav aria-label="Markdown outline" className="text-sm">
          {headings.map((heading) => (
            <a
              key={heading}
              href={`#${slug(heading)}`}
              className="block rounded-lg px-2 py-1 font-semibold text-indigo-600 transition-colors hover:bg-indigo-50 hover:text-orange-700 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200"
            >
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

  return (
    <pre className="max-h-[34rem] overflow-auto p-3 text-xs leading-5 text-indigo-900">
      {content}
    </pre>
  );
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
    <div className="space-y-2 p-3 text-xs text-indigo-900">
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
      <strong className="text-indigo-950">{name}</strong>
      {long && !open ? (
        <button
          type="button"
          aria-label={`展开 ${name}`}
          className="ml-2 rounded-lg px-1 font-bold text-orange-700 hover:bg-orange-100 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200"
          onClick={() => setOpen(true)}
        >
          展开
        </button>
      ) : null}
      <pre className="mt-1 rounded-lg border-2 border-indigo-100 bg-indigo-50/80 p-2 text-indigo-900">
        {long && !open ? `${text.length} chars hidden` : text}
      </pre>
    </div>
  );
}
