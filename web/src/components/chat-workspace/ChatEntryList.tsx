import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { RevisionPath } from "../../api/types";
import type { ChatEntry, ChoiceResponsePayload, WorkspaceContentRef } from "../../state/chat-entries";
import { ChatEntryRenderer } from "./ChatEntryRenderer";
import { MessageGroupView } from "./MessageGroupView";
import { groupEntries, type MessageGroup } from "./message-grouping";

export interface ChatEntryListHandle {
  scrollToEntry: (entryId: string) => void;
}

interface ChatEntryListProps {
  entries: ChatEntry[];
  onPermissionResponse?: (entry: ChatEntry, approved: boolean) => void;
  onChoiceResponse?: (entry: ChatEntry, response: ChoiceResponsePayload) => void;
  onSelectRevisionPath?: (path: RevisionPath, extraContext?: string) => void;
  onHumanConfirm?: (
    decision: "confirm" | "request-change" | "terminate",
    payload?: unknown,
  ) => void;
  sessionId?: string | null;
  contentCache?: Record<string, string>;
  loadContent?: (sessionId: string, ref: WorkspaceContentRef) => Promise<string>;
  onCacheContent?: (key: string, value: string) => void;
  className?: string;
}

export const ChatEntryList = forwardRef<ChatEntryListHandle, ChatEntryListProps>(
  function ChatEntryList(
    {
      entries,
      onPermissionResponse,
      onChoiceResponse,
      onSelectRevisionPath,
      onHumanConfirm,
      sessionId,
      contentCache,
      loadContent,
      onCacheContent,
      className = "",
    },
    ref,
  ) {
    const parentRef = useRef<HTMLDivElement | null>(null);
    const [scrollElement, setScrollElement] = useState<HTMLDivElement | null>(null);
    const latestEntryId = entries.at(-1)?.id ?? null;
    const groupedItems = useMemo(() => groupEntries(entries), [entries]);
    const entryIndexById = useMemo(() => {
      const map = new Map<string, number>();
      groupedItems.forEach((item, index) => {
        if (item.kind === "group") {
          map.set(entryIdForGroup(item.group), index);
        } else {
          map.set(item.entry.id, index);
        }
      });
      return map;
    }, [groupedItems]);
    const rowVirtualizer = useVirtualizer({
      count: groupedItems.length,
      getScrollElement: () => scrollElement,
      getItemKey: (index) => {
        const item = groupedItems[index];
        if (!item) {
          return index;
        }
        return item.kind === "group" ? entryIdForGroup(item.group) : item.entry.id;
      },
      estimateSize: () => 140,
      overscan: 6,
      initialRect: { width: 0, height: 800 },
      observeElementRect: (_instance, callback) => {
        callback({ width: scrollElement?.clientWidth ?? 0, height: scrollElement?.clientHeight || 800 });
        const observer = new ResizeObserver(([entry]) => {
          callback({
            width: entry?.contentRect.width ?? scrollElement?.clientWidth ?? 0,
            height: entry?.contentRect.height || scrollElement?.clientHeight || 800,
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
        const handleScroll = () => callback(scrollElement.scrollTop, false);
        scrollElement.addEventListener("scroll", handleScroll, { passive: true });
        return () => scrollElement.removeEventListener("scroll", handleScroll);
      },
    });

    useImperativeHandle(
      ref,
      () => ({
        scrollToEntry(entryId: string) {
          const index = entryIndexById.get(entryId);
          if (index !== undefined) {
            rowVirtualizer.scrollToIndex(index, { align: "start" });
            parentRef.current
              ?.querySelector<HTMLElement>(`[data-entry-id="${entryId}"]`)
              ?.scrollIntoView({ behavior: "auto", block: "start" });
          }
        },
      }),
      [entryIndexById, rowVirtualizer],
    );

    useEffect(() => {
      if (groupedItems.length > 0) {
        rowVirtualizer.scrollToIndex(groupedItems.length - 1, { align: "end" });
      }
    }, [groupedItems.length, latestEntryId, rowVirtualizer]);

    return (
      <div
        ref={(node) => {
          parentRef.current = node;
          setScrollElement(node);
        }}
        data-testid="chat-entry-list"
        className={[
          "min-h-0 overflow-auto px-3 py-4",
          entries.length === 0 ? "flex items-center justify-center" : "",
          className,
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {entries.length === 0 ? (
          <div className="text-sm text-[var(--aria-ink-muted)]">暂无聊天记录</div>
        ) : (
          <div
            className="relative w-full"
            style={{ height: `${rowVirtualizer.getTotalSize()}px` }}
          >
            {rowVirtualizer.getVirtualItems().map((virtualRow) => {
              const item = groupedItems[virtualRow.index];
              if (!item) {
                return null;
              }

              const rowClassName = "absolute left-0 top-0 w-full min-w-0 pb-3";
              const rowStyle = { transform: `translateY(${virtualRow.start}px)` };

              if (item.kind === "group") {
                return (
                  <div
                    key={virtualRow.key}
                    ref={rowVirtualizer.measureElement}
                    data-index={virtualRow.index}
                    data-entry-id={entryIdForGroup(item.group)}
                    className={rowClassName}
                    style={rowStyle}
                  >
                    <MessageGroupView
                      group={item.group}
                      onPermissionResponse={onPermissionResponse}
                      onChoiceResponse={onChoiceResponse}
                      onSelectRevisionPath={onSelectRevisionPath}
                      onHumanConfirm={onHumanConfirm}
                      sessionId={sessionId}
                      contentCache={contentCache}
                      loadContent={loadContent}
                      onCacheContent={onCacheContent}
                    />
                  </div>
                );
              }

              return (
                <div
                  key={virtualRow.key}
                  ref={rowVirtualizer.measureElement}
                  data-index={virtualRow.index}
                  data-entry-id={item.entry.id}
                  className={rowClassName}
                  style={rowStyle}
                >
                  <ChatEntryRenderer
                    entry={item.entry}
                    onPermissionResponse={onPermissionResponse}
                    onChoiceResponse={onChoiceResponse}
                    onSelectRevisionPath={onSelectRevisionPath}
                    onHumanConfirm={onHumanConfirm}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>
    );
  },
);

function entryIdForGroup(group: MessageGroup) {
  return (
    group.primaryEntry?.id ??
    group.inlineEvents[0]?.id ??
    group.interruptEntries[0]?.id ??
    group.id
  );
}
