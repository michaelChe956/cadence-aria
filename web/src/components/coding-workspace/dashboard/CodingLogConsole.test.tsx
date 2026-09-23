import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { CODING_LOG_TAIL_LIMIT, useCodingLogStore, type CodingLogEntryInput } from "../../../state/coding-log-store";
import { CodingLogConsole } from "./CodingLogConsole";

function line(overrides: Partial<CodingLogEntryInput> = {}): CodingLogEntryInput {
  return {
    nodeId: "node_1",
    nodeTitle: "Coder",
    text: "sample line",
    at: "2026-09-14T08:00:01.000Z",
    kind: "stream",
    ...overrides,
  };
}

describe("CodingLogConsole", () => {
  beforeEach(() => {
    useCodingLogStore.getState().reset();
  });

  it("merges lines by node and filters on node selection", async () => {
    const user = userEvent.setup();
    act(() => {
      useCodingLogStore.getState().appendLines([
        line(),
        line({ nodeId: "node_2", nodeTitle: "Reviewer", text: "review line" }),
        line({ text: "second coder line" }),
      ]);
    });
    render(<CodingLogConsole />);
    expect(screen.getAllByTestId("coding-log-line")).toHaveLength(3);
    await user.selectOptions(screen.getByLabelText("按节点筛选日志"), "node_1");
    expect(screen.getAllByTestId("coding-log-line")).toHaveLength(2);
    expect(screen.queryByText("review line")).toBeNull();
    await user.selectOptions(screen.getByLabelText("按节点筛选日志"), "");
    expect(screen.getAllByTestId("coding-log-line")).toHaveLength(3);
  });

  it("renders only a viewport-sized window of a long log (virtualization)", () => {
    act(() => {
      useCodingLogStore.getState().appendLines(
        Array.from({ length: CODING_LOG_TAIL_LIMIT }, (_, index) => line({ text: `log-${index}` })),
      );
    });
    const { container } = render(<CodingLogConsole />);
    const rendered = container.querySelectorAll('[data-index]');
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered.length).toBeLessThan(CODING_LOG_TAIL_LIMIT);
  });

  it("caps the scroll container with an explicit max-height so its height stays content-independent", () => {
    act(() => {
      useCodingLogStore.getState().appendLines([line({ text: "only" })]);
    });
    const { container } = render(<CodingLogConsole />);
    const scroller = container.querySelector('[data-testid="coding-log-console-scroll"]') as HTMLElement;
    // jsdom 无布局,断言定高类存在:长日志下滚动容器高度不得跟随内容(否则虚拟化失效、外层被撑破)。
    expect(scroller.className).toContain("max-h-[40vh]");
  });

  it("shows an explicit empty state when no line matches", () => {
    render(<CodingLogConsole />);
    expect(screen.getByTestId("coding-log-empty")).toHaveTextContent("暂无日志");
  });

  // F-43 ④：日志行此前「绝对定位 + 写死 20px 行高 + truncate 与
  // whitespace-pre-wrap 互相覆盖」——多行文本在定高行里换行后溢出到下一行
  // （文字叠印），节点名 shrink-0 又把长串撑出横向滚动。契约：行高交给
  // 虚拟化按内容测量（不再写死），消息 break-words 单次渲染、节点名限宽可缩。
  it("lays each log line out as a measured single copy without a fixed row height", () => {
    const longLine = `${"x".repeat(400)} tail`;
    act(() => {
      useCodingLogStore.getState().appendLines([
        line({ text: longLine }),
        line({ kind: "event", text: "event line" }),
      ]);
    });

    render(<CodingLogConsole />);

    const rows = screen.getAllByTestId("coding-log-line");
    const streamRow = rows[0]!;
    const eventRow = rows[1]!;

    // 行高不再写死：多行内容由测量接管，不会压到相邻行。
    expect(streamRow.style.height).toBe("");
    expect(streamRow).toHaveAttribute("data-index", "0");
    expect(eventRow.style.height).toBe("");

    // 单印：同一行文本在 DOM 里只出现一次。
    expect(within(streamRow).getAllByText(longLine)).toHaveLength(1);

    const message = within(streamRow).getByText(longLine);
    expect(message.className).toContain("break-words");
    expect(message.className).not.toContain("truncate");

    const eventMessage = within(eventRow).getByText("event line");
    expect(eventMessage.className).not.toContain("truncate");

    // 横向不溢出：节点名限宽可收缩，不再 shrink-0 撑破行长。
    const nodeTitle = within(streamRow).getByText("Coder");
    expect(nodeTitle.className).not.toContain("shrink-0");
    expect(nodeTitle.className).toContain("max-w-");
  });

  it("keeps the newest line visible when pinned to the bottom", () => {
    act(() => {
      useCodingLogStore.getState().appendLines([line({ text: "first" })]);
    });
    const { container } = render(<CodingLogConsole />);
    const scroller = container.querySelector('[data-testid="coding-log-console-scroll"]') as HTMLElement;
    scroller.scrollTop = scroller.scrollHeight; // 用户已贴底
    act(() => {
      useCodingLogStore.getState().appendLines([line({ text: "newest" })]);
    });
    expect(scroller.scrollTop + scroller.clientHeight).toBeGreaterThanOrEqual(scroller.scrollHeight - 24);
  });

  it("keeps pinning when appending past the tail limit keeps the length stable", () => {
    act(() => {
      useCodingLogStore.getState().appendLines(
        Array.from({ length: CODING_LOG_TAIL_LIMIT }, (_, index) => line({ text: `log-${index}` })),
      );
    });
    const { container } = render(<CodingLogConsole />);
    const scroller = container.querySelector('[data-testid="coding-log-console-scroll"]') as HTMLElement;
    // jsdom 无布局:自管 scrollTop 并让 scrollHeight 随新内容递增,断言贴底写入跟随新底。
    let scrollTop = 0;
    let totalHeight = 1000;
    Object.defineProperty(scroller, "scrollTop", {
      configurable: true,
      get: () => scrollTop,
      set: (value: number) => {
        scrollTop = value;
      },
    });
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      get: () => totalHeight,
    });
    scrollTop = totalHeight; // 用户已贴底
    totalHeight = 1200; // 新行同步布局后总高递增(effect 运行前须已就绪)
    act(() => {
      useCodingLogStore.getState().appendLines([line({ text: "newest" })]);
    });
    expect(useCodingLogStore.getState().lines).toHaveLength(CODING_LOG_TAIL_LIMIT); // 长度稳态
    expect(scroller.scrollTop).toBe(1200); // 尾行变化仍触发贴底
  });
});
