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
    // F-43 ④：行高不再写死 20px（定高 + 换行内容 = 相邻行文字叠印）。行元素交给
    // 虚拟化按内容测量；jsdom/无布局环境量到 0 时回退估算行高，保持既有窗口化行为。
    measureElement: (element, entry) => {
      const measured =
        entry?.borderBoxSize?.[0]?.blockSize ?? (element as HTMLElement).offsetHeight;
      return measured > 0 ? measured : LINE_HEIGHT;
    },
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
          <span className="aria-num text-[10px] font-normal text-slate-600">
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
        <div data-testid="coding-log-empty" className="text-xs text-slate-600">
          暂无日志
        </div>
      ) : (
        <div
          ref={setScrollElement}
          data-testid="coding-log-console-scroll"
          onScroll={handleScroll}
          className="min-h-0 max-h-[40vh] overflow-auto rounded-md bg-[var(--aria-panel-subtle)]"
        >
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const line = visibleLines[item.index];
              if (!line) return null;
              return (
                <div
                  key={line.id}
                  ref={virtualizer.measureElement}
                  data-index={item.index}
                  data-testid="coding-log-line"
                  className="absolute left-0 top-0 flex w-full min-w-0 items-baseline gap-2 overflow-hidden px-2 font-mono text-[11px] leading-5"
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  <span className="aria-num shrink-0 text-slate-600">{line.at.slice(11, 19)}</span>
                  {/* 节点名限宽可收缩：shrink-0 + 无上限会把行撑出容器（横向溢出）。 */}
                  <span className="min-w-0 max-w-[8rem] shrink truncate text-slate-600">
                    {line.nodeTitle ?? "全局"}
                  </span>
                  {/* 消息允许按词换行（break-words + pre-wrap），行高由测量承接；不再叠加
                      truncate 的 nowrap——两者同时存在时换行内容会溢出定高行压住下一行。 */}
                  <span
                    className={
                      line.kind === "event"
                        ? "min-w-0 flex-1 whitespace-pre-wrap break-words border-l-2 border-[var(--aria-line-strong)] pl-1.5 text-[var(--aria-ink)]"
                        : "min-w-0 flex-1 whitespace-pre-wrap break-words text-[var(--aria-ink)]"
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
