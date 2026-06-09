import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../state/chat-entries";
import { ChatEntryList, type ChatEntryListHandle } from "./ChatEntryList";

const scrollIntoView = vi.fn();
const scrollToIndex = vi.hoisted(() => vi.fn());
const measureElement = vi.hoisted(() => vi.fn());
const virtualItemKeys = vi.hoisted(() => [] as Array<string | number>);

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({
    count,
    getItemKey,
  }: {
    count: number;
    getItemKey?: (index: number) => string | number;
  }) => {
    const items = Array.from({ length: count }, (_, index) => {
      const key = getItemKey?.(index) ?? index;
      virtualItemKeys.push(key);
      return {
        index,
        key,
        start: index * 140,
        size: 140,
      };
    });

    return {
      getTotalSize: () => count * 140,
      getVirtualItems: () => items,
      measureElement,
      scrollToIndex,
    };
  },
}));

describe("ChatEntryList", () => {
  beforeEach(() => {
    scrollIntoView.mockClear();
    scrollToIndex.mockClear();
    measureElement.mockClear();
    virtualItemKeys.length = 0;
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
  });

  it("renders entries and scrolls to the latest item automatically", () => {
    const { rerender } = render(<ChatEntryList entries={[]} />);

    rerender(<ChatEntryList entries={[makeEntry("entry-1", "context_note", "第一条")]} />);
    rerender(
      <ChatEntryList
        entries={[
          makeEntry("entry-1", "context_note", "第一条"),
          makeEntry("entry-2", "provider_stream", "第二条"),
        ]}
      />,
    );

    expect(screen.getByText("第一条")).toBeInTheDocument();
    expect(screen.getByText("第二条")).toBeInTheDocument();
    expect(virtualItemKeys).toContain("entry-1");
    expect(virtualItemKeys).toContain("entry-2");
    expect(scrollToIndex).toHaveBeenCalledWith(1, { align: "end" });
  });

  it("measures virtual rows by data-index without forcing fixed row height", () => {
    render(
      <ChatEntryList
        entries={[
          makeEntry("entry-1", "stage_change", "第一条"),
          makeEntry("entry-2", "stage_change", "第二条"),
        ]}
      />,
    );

    const firstRow = screen.getByText("第一条").closest("[data-entry-id='entry-1']");

    expect(firstRow).toHaveAttribute("data-index", "0");
    expect(firstRow).not.toHaveStyle({ height: "140px" });
    expect(measureElement).toHaveBeenCalledWith(firstRow);
  });

  it("scrolls to a specific entry on demand", () => {
    const ref = createRef<ChatEntryListHandle>();

    render(
      <ChatEntryList
        ref={ref}
        entries={[
          makeEntry("entry-1", "context_note", "第一条"),
          makeEntry("entry-2", "provider_stream", "第二条"),
        ]}
      />,
    );

    scrollIntoView.mockClear();
    ref.current?.scrollToEntry("entry-2");

    expect(scrollToIndex).toHaveBeenCalledWith(1, { align: "start" });
  });

  it("scrolls requested timeline targets immediately for long review threads", () => {
    const ref = createRef<ChatEntryListHandle>();

    render(
      <ChatEntryList
        ref={ref}
        entries={[
          makeEntry("round-1-author", "provider_stream", "第一轮作者输出"),
          makeEntry("round-2-author", "provider_stream", "第二轮作者输出"),
          makeEntry("round-3-author-confirm", "stage_change", "Author 结果确认 · 已进入 Review"),
        ]}
      />,
    );

    scrollIntoView.mockClear();
    ref.current?.scrollToEntry("round-3-author-confirm");

    expect(scrollToIndex).toHaveBeenCalledWith(1, { align: "start" });
  });

  it("scrolls to an execution-only grouped entry on demand", () => {
    const ref = createRef<ChatEntryListHandle>();

    render(
      <ChatEntryList
        ref={ref}
        entries={[
          {
            ...makeEntry("entry-event", "execution_event", "命令失败"),
            node_id: "node-failed",
          },
        ]}
      />,
    );

    scrollIntoView.mockClear();
    ref.current?.scrollToEntry("entry-event");

    expect(scrollToIndex).toHaveBeenCalledWith(0, { align: "start" });
  });

  it("keeps scrollToEntry available for timeline selection", () => {
    const ref = createRef<ChatEntryListHandle>();
    const entries = Array.from({ length: 100 }, (_, index) => ({
      id: `entry-${index}`,
      type: "stage_change" as const,
      role: "system" as const,
      content: `Entry ${index}`,
      timestamp: "2026-06-06T00:00:00Z",
    }));

    render(<ChatEntryList ref={ref} entries={entries} />);

    expect(() => ref.current?.scrollToEntry("entry-80")).not.toThrow();
    expect(scrollToIndex).toHaveBeenCalledWith(80, { align: "start" });
  });
});

function makeEntry(id: string, type: ChatEntry["type"], content: string): ChatEntry {
  return {
    id,
    type,
    role: type === "context_note" ? "user" : "author",
    content,
    timestamp: "2026-05-21T10:00:00Z",
  };
}
