import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ScrollText } from "lucide-react";
import {
  codingLogNodeOptions,
  filterCodingLogLines,
  useCodingLogStore,
} from "../../../state/coding-log-store";

const LINE_HEIGHT = 20;
const PIN_THRESHOLD_PX = 24;

export function CodingLogConsole() {
  const lines = useCodingLogStore((state) => state.lines);
  const [nodeFilter, setNodeFilter] = useState<string>(""); // "" = 全部节点
  const scrollElementRef = useRef<HTMLDivElement | null>(null);
  const [scrollElement, setScrollElement] = useState<HTMLDivElement | null>(null);
  const pinnedRef = useRef(true);

  const visibleLines = useMemo(
    () => filterCodingLogLines(lines, nodeFilter === "" ? null : nodeFilter),
    [lines, nodeFilter],
  );
  const nodeOptions = useMemo(() => codingLogNodeOptions(lines), [lines]);
  const latestLineId = visibleLines.at(-1)?.id ?? null; // 稳态(length 被 tail 上限钉死)仍随尾行变化
  const virtualizer = useVirtualizer({
    count: visibleLines.length,
    getScrollElement: () => scrollElement,
    estimateSize: () => LINE_HEIGHT,
    overscan: 10,
    getItemKey: (index) => visibleLines[index]?.id ?? index,
    initialRect: { width: 0, height: 320 },
    // 对齐 ChatEntryList 的接线:jsdom/无布局环境 clientHeight 为 0,
    // 默认 observer 会把 scrollRect 钉在 {0,0} 导致 range 恒空,这里回退固定高度。
    observeElementRect: (_instance, callback) => {
      callback({ width: scrollElement?.clientWidth ?? 0, height: scrollElement?.clientHeight || 320 });
      const observer = new ResizeObserver(([entry]) => {
        callback({
          width: entry?.contentRect.width ?? scrollElement?.clientWidth ?? 0,
          height: entry?.contentRect.height || scrollElement?.clientHeight || 320,
        });
      });
      if (scrollElement) {
        observer.observe(scrollElement);
      }
      return () => observer.disconnect();
    },
    observeElementOffset: (_instance, callback) => {
      callback(scrollElement?.scrollTop ?? 0, false);
      if (!scrollElement) {
        return () => undefined;
      }
      const handleOffsetChange = () => callback(scrollElement.scrollTop, false);
      scrollElement.addEventListener("scroll", handleOffsetChange, { passive: true });
      return () => scrollElement.removeEventListener("scroll", handleOffsetChange);
    },
  });

  useEffect(() => {
    if (!scrollElement || !pinnedRef.current) return;
    scrollElement.scrollTop = scrollElement.scrollHeight;
  }, [scrollElement, virtualizer, visibleLines.length, latestLineId]);

  function handleScroll() {
    if (!scrollElement) return;
    pinnedRef.current =
      scrollElement.scrollTop + scrollElement.clientHeight >= scrollElement.scrollHeight - PIN_THRESHOLD_PX;
  }

  return (
    <section data-testid="coding-log-console" aria-label="实时日志" className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-2 rounded-lg border border-[var(--aria-line)] bg-white p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-[var(--aria-ink)]">
          <ScrollText className="h-4 w-4" aria-hidden="true" />
          实时日志
          <span className="aria-num text-[10px] font-normal text-[var(--aria-ink-muted)]">
            {visibleLines.length}/{lines.length}
          </span>
        </div>
        <select
          aria-label="按节点筛选日志"
          value={nodeFilter}
          onChange={(event) => setNodeFilter(event.target.value)}
          className="min-h-11 max-w-[10rem] rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs text-[var(--aria-ink)] focus-visible:outline-2 focus-visible:outline-[var(--aria-primary)]"
        >
          <option value="">全部节点</option>
          {nodeOptions.map((option) => (
            <option key={option.nodeId} value={option.nodeId}>
              {option.nodeTitle}
            </option>
          ))}
        </select>
      </div>
      {visibleLines.length === 0 ? (
        <div data-testid="coding-log-empty" className="text-xs text-[var(--aria-ink-muted)]">
          暂无日志
        </div>
      ) : (
        <div
          ref={(node) => {
            scrollElementRef.current = node;
            setScrollElement(node);
          }}
          data-testid="coding-log-console-scroll"
          onScroll={handleScroll}
          className="min-h-0 overflow-auto rounded-md bg-[var(--aria-panel-subtle)]"
        >
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const line = visibleLines[item.index];
              if (!line) return null;
              return (
                <div
                  key={line.id}
                  data-index={item.index}
                  data-testid="coding-log-line"
                  className="absolute left-0 top-0 flex w-full items-baseline gap-2 px-2 font-mono text-[11px] leading-5"
                  style={{ transform: `translateY(${item.start}px)`, height: LINE_HEIGHT }}
                >
                  <span className="aria-num shrink-0 text-[var(--aria-ink-muted)]">{line.at.slice(11, 19)}</span>
                  <span className="min-w-0 shrink-0 truncate text-[var(--aria-ink-muted)]">
                    {line.nodeTitle ?? "全局"}
                  </span>
                  <span
                    className={
                      line.kind === "event"
                        ? "min-w-0 flex-1 truncate border-l-2 border-[var(--aria-line-strong)] pl-1.5 text-[var(--aria-ink)]"
                        : "min-w-0 flex-1 truncate whitespace-pre-wrap break-all text-[var(--aria-ink)]"
                    }
                  >
                    {line.text}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </section>
  );
}
