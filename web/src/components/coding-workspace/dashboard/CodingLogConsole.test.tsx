import { act, render, screen } from "@testing-library/react";
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

  it("shows an explicit empty state when no line matches", () => {
    render(<CodingLogConsole />);
    expect(screen.getByTestId("coding-log-empty")).toHaveTextContent("暂无日志");
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
});
