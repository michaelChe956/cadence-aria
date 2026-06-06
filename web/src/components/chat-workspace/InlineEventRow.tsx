import { ChevronDown, ChevronRight, Wrench } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ChatEntry, WorkspaceContentRef } from "../../state/chat-entries";
import { workspaceContentCacheKey } from "../../state/workspace-ws-store";

interface InlineEventRowProps {
  entry: ChatEntry;
  sessionId?: string | null;
  contentCache?: Record<string, string>;
  loadContent?: (sessionId: string, ref: WorkspaceContentRef) => Promise<string>;
  onCacheContent?: (key: string, value: string) => void;
}

export function InlineEventRow({
  entry,
  sessionId,
  contentCache = {},
  loadContent,
  onCacheContent,
}: InlineEventRowProps) {
  const [expanded, setExpanded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlightKeyRef = useRef<string | null>(null);
  const mountedRef = useRef(true);
  const event = entry.metadata as Record<string, unknown> | undefined;
  const command = typeof event?.command === "string" ? event.command : null;
  const metadataOutput = typeof event?.output === "string" ? event.output : null;
  const detail = typeof event?.detail === "string" ? event.detail : null;
  const cacheKey = entry.content_ref ? workspaceContentCacheKey(entry.content_ref) : null;
  const cachedOutput = cacheKey ? contentCache[cacheKey] : undefined;
  const output = metadataOutput ?? cachedOutput ?? null;
  const loadableRef = entry.content_ref?.kind === "execution_output" || entry.content_ref?.kind === "provider_prompt" ? entry.content_ref : null;
  const shouldLoadExecutionOutput =
    loadableRef !== null && !metadataOutput && !cachedOutput;

  useEffect(() => {
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!expanded || !shouldLoadExecutionOutput) {
      return;
    }
    if (!sessionId || !loadContent || !cacheKey) {
      return;
    }
    if (inFlightKeyRef.current === cacheKey) {
      return;
    }

    inFlightKeyRef.current = cacheKey;
    setLoading(true);
    setError(null);
    loadContent(sessionId, loadableRef)
      .then((value) => {
        onCacheContent?.(cacheKey, value);
      })
      .catch((cause: unknown) => {
        if (!mountedRef.current) {
          return;
        }
        setError(cause instanceof Error ? cause.message : "加载输出失败");
      })
      .finally(() => {
        if (mountedRef.current) {
          setLoading(false);
        }
        if (inFlightKeyRef.current === cacheKey) {
          inFlightKeyRef.current = null;
        }
      });
  }, [cacheKey, expanded, loadContent, loadableRef, onCacheContent, sessionId, shouldLoadExecutionOutput]);

  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]">
      <button
        type="button"
        onClick={() => setExpanded((current) => !current)}
        className="flex min-h-9 w-full min-w-0 items-center gap-2 px-2 text-left text-xs font-medium text-[var(--aria-ink)] hover:bg-white"
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-[var(--aria-ink-muted)]" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-[var(--aria-ink-muted)]" />
        )}
        <Wrench className="h-3.5 w-3.5 shrink-0 text-[var(--aria-primary)]" />
        <span className="min-w-0 truncate">{entry.content}</span>
      </button>
      {expanded ? (
        <div className="space-y-2 border-t border-[var(--aria-line)] px-2 py-2">
          {detail ? <div className="text-xs text-[var(--aria-ink-muted)]">{detail}</div> : null}
          {command ? (
            <div className="rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
              {command}
            </div>
          ) : null}
          {output ? (
            <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink)]">
              {output}
            </pre>
          ) : null}
          {loading ? <div className="text-xs text-[var(--aria-ink-muted)]">加载输出中...</div> : null}
          {error ? <div className="text-xs text-red-700">{error}</div> : null}
          {!output && !loading && !error ? (
            <div className="text-xs text-[var(--aria-ink-muted)]">暂无输出</div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
