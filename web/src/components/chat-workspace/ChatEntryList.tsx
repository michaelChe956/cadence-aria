import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef } from "react";
import type { RevisionPath } from "../../api/types";
import type { ChatEntry, ChoiceResponsePayload } from "../../state/chat-entries";
import { ChatEntryRenderer } from "./ChatEntryRenderer";
import { MessageGroupView } from "./MessageGroupView";
import { groupEntries } from "./message-grouping";

export interface ChatEntryListHandle {
  scrollToEntry: (entryId: string) => void;
}

interface ChatEntryListProps {
  entries: ChatEntry[];
  onPermissionResponse?: (entry: ChatEntry, approved: boolean) => void;
  onChoiceResponse?: (entry: ChatEntry, response: ChoiceResponsePayload) => void;
  onSelectRevisionPath?: (path: RevisionPath, extraContext?: string) => void;
  onHumanConfirm?: (decision: "confirm" | "request-change" | "terminate") => void;
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
      className = "",
    },
    ref,
  ) {
    const listRef = useRef<HTMLDivElement | null>(null);
    const endRef = useRef<HTMLDivElement | null>(null);
    const latestEntryId = entries.at(-1)?.id ?? null;
    const groupedItems = useMemo(() => groupEntries(entries), [entries]);

    useImperativeHandle(
      ref,
      () => ({
        scrollToEntry(entryId: string) {
          const target = listRef.current?.querySelector<HTMLElement>(
            `[data-entry-id="${entryId}"]`,
          );
          target?.scrollIntoView({ behavior: "smooth", block: "start" });
        },
      }),
      [],
    );

    useEffect(() => {
      endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }, [entries.length, latestEntryId]);

    return (
      <div
        ref={listRef}
        data-testid="chat-entry-list"
        className={[
          "min-h-0 overflow-auto px-3 py-4",
          entries.length === 0 ? "flex items-center justify-center" : "space-y-3",
          className,
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {entries.length === 0 ? (
          <div className="text-sm text-[var(--aria-ink-muted)]">暂无聊天记录</div>
        ) : (
          groupedItems.map((item) => {
            if (item.kind === "group") {
              return (
                <div
                  key={item.group.id}
                  data-entry-id={item.group.primaryEntry?.id ?? item.group.id}
                  className="min-w-0"
                >
                  <MessageGroupView
                    group={item.group}
                    onPermissionResponse={onPermissionResponse}
                    onChoiceResponse={onChoiceResponse}
                    onSelectRevisionPath={onSelectRevisionPath}
                    onHumanConfirm={onHumanConfirm}
                  />
                </div>
              );
            }
            return (
              <div key={item.entry.id} data-entry-id={item.entry.id} className="min-w-0">
                <ChatEntryRenderer
                  entry={item.entry}
                  onPermissionResponse={onPermissionResponse}
                  onChoiceResponse={onChoiceResponse}
                  onSelectRevisionPath={onSelectRevisionPath}
                  onHumanConfirm={onHumanConfirm}
                />
              </div>
            );
          })
        )}
        <div ref={endRef} aria-hidden="true" className="h-px" />
      </div>
    );
  },
);
