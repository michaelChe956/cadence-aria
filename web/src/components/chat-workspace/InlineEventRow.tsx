import { ChevronRight } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ChatEntry, WorkspaceContentRef } from "../../state/chat-entries";
import { workspaceContentCacheKey } from "../../state/workspace-ws-store";
import { executionEventIcon } from "./gate-visual-tokens";
import { normalizeDisplayText } from "./text-display";

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
  const isProviderPrompt =
    entry.content_ref?.kind === "provider_prompt" || event?.title === "Provider Prompt";
  const cacheKey = entry.content_ref ? workspaceContentCacheKey(entry.content_ref) : null;
  const cachedOutput = cacheKey ? contentCache[cacheKey] : undefined;
  const output = metadataOutput ?? cachedOutput ?? null;
  const loadableRef =
    entry.content_ref?.kind === "execution_output" || entry.content_ref?.kind === "provider_prompt"
      ? entry.content_ref
      : null;
  const shouldLoadExecutionOutput =
    loadableRef !== null && !metadataOutput && !cachedOutput;
  const displayContent = normalizeDisplayText(entry.content);
  const displayDetail = detail ? normalizeDisplayText(detail) : null;
  const displayCommand = command ? normalizeDisplayText(command) : null;
  const displayOutput = output ? normalizeDisplayText(output) : null;
  // F-50 视觉 v2 §4：emoji 图标退役——事件 kind 映射 Lucide SVG（provider→zap /
  // usage→gauge / command/output→terminal），统一 16px stroke-current。
  const EventIcon = executionEventIcon(
    typeof event?.kind === "string" ? event.kind : undefined,
    isProviderPrompt,
  );
  const loadingText = isProviderPrompt ? "加载 Prompt 中..." : "加载输出中...";
  const emptyText = isProviderPrompt ? "暂无 Prompt 内容" : "暂无输出";
  const outputMaxHeight = isProviderPrompt ? "max-h-96" : "max-h-40";

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
    <div
      className={[
        "rounded-md border border-[var(--aria-line)]",
        isProviderPrompt ? "bg-white" : "bg-[var(--aria-panel-muted)]",
      ].join(" ")}
    >
      <button
        type="button"
        onClick={() => setExpanded((current) => !current)}
        className="flex min-h-9 w-full min-w-0 cursor-pointer items-center gap-2 px-2 text-left text-xs font-medium text-slate-900 transition-colors duration-150 hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400"
      >
        {/* ▶ 折叠→chevron-right 旋转过渡（不再切换两个图标）；减弱动效静止。 */}
        <ChevronRight
          className={`h-4 w-4 shrink-0 text-slate-500 transition-transform duration-200 motion-reduce:transition-none ${
            expanded ? "rotate-90" : ""
          }`}
          aria-hidden="true"
        />
        <EventIcon className="h-4 w-4 shrink-0 text-slate-600" aria-hidden="true" />
        {isProviderPrompt ? (
          <span className="shrink-0 rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--aria-ink-muted)]">
            PROMPT
          </span>
        ) : null}
        <span className="min-w-0 truncate">{displayContent}</span>
      </button>
      {expanded ? (
        <div className="space-y-2 border-t border-[var(--aria-line)] px-2 py-2">
          {displayDetail ? (
            <div className="text-xs text-[var(--aria-ink-muted)]">{displayDetail}</div>
          ) : null}
          {displayCommand ? (
            <div className="rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
              {displayCommand}
            </div>
          ) : null}
          {displayOutput ? (
            <pre
              className={`${outputMaxHeight} overflow-auto whitespace-pre-wrap rounded bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink)]`}
            >
              {displayOutput}
            </pre>
          ) : null}
          {loading ? <div className="text-xs text-[var(--aria-ink-muted)]">{loadingText}</div> : null}
          {error ? <div className="text-xs text-red-700">{error}</div> : null}
          {!displayOutput && !loading && !error ? (
            <div className="text-xs text-[var(--aria-ink-muted)]">{emptyText}</div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
