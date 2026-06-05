import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../state/chat-entries";
import { ChatEntryList, type ChatEntryListHandle } from "./ChatEntryList";

const scrollIntoView = vi.fn();

describe("ChatEntryList", () => {
  beforeEach(() => {
    scrollIntoView.mockClear();
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
    expect(scrollIntoView).toHaveBeenCalled();
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

    expect(scrollIntoView).toHaveBeenCalledTimes(1);
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

    expect(scrollIntoView).toHaveBeenCalledWith({ behavior: "auto", block: "start" });
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

    expect(scrollIntoView).toHaveBeenCalledTimes(1);
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
