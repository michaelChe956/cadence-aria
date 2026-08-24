// Task 3（展示组件）：IssueQueueRow 与 StageMiniGraph 的渲染单测。
// 视觉契约与 Plan 逐字一致：4 个 pip 及 data-state、focused 左条/背景/aria-current、
// 操作按钮 opacity-0 默认隐藏、点击标题触发 onSelect。
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { IssueQueueRowData, StagePip } from "./issue-queue-derivation";
import { IssueQueueRow, StageMiniGraph } from "./IssueQueueRow";

// 覆盖全部四种 pip 状态的 fixture（展示组件只按传入 pips 渲染，不关心分组推导）。
function queueRow(overrides: Partial<IssueQueueRowData> = {}): IssueQueueRowData {
  return {
    issueId: "issue_0001",
    title: "会话过期提示",
    status: "draft",
    stagePips: [
      { stage: "story", state: "done" },
      { stage: "design", state: "active" },
      { stage: "work_item", state: "blocked" },
      { stage: "coding", state: "pending" },
    ],
    group: "blocked",
    storyCount: 1,
    designCount: 0,
    workItemCount: 1,
    ...overrides,
  };
}

describe("StageMiniGraph", () => {
  it("渲染 4 个 pip 及 data-state，按 story/design/work_item/coding 顺序", () => {
    render(<StageMiniGraph pips={queueRow().stagePips} />);

    const graph = screen.getByTestId("stage-mini-graph");
    expect(graph.querySelectorAll("[data-state]")).toHaveLength(4);

    expect(screen.getByTestId("stage-pip-story")).toHaveAttribute(
      "data-state",
      "done",
    );
    expect(screen.getByTestId("stage-pip-design")).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.getByTestId("stage-pip-work_item")).toHaveAttribute(
      "data-state",
      "blocked",
    );
    expect(screen.getByTestId("stage-pip-coding")).toHaveAttribute(
      "data-state",
      "pending",
    );

    // pip 顺序：story -> design -> work_item -> coding
    expect(
      Array.from(graph.querySelectorAll("[data-state]")).map(
        (pip) => pip.getAttribute("data-testid"),
      ),
    ).toEqual([
      "stage-pip-story",
      "stage-pip-design",
      "stage-pip-work_item",
      "stage-pip-coding",
    ]);
  });

  it("pip 为 h-2 w-2 rounded-full 且状态色映射正确", () => {
    render(<StageMiniGraph pips={queueRow().stagePips} />);

    const pipByState: Record<string, string> = {
      done: "stage-pip-story",
      active: "stage-pip-design",
      blocked: "stage-pip-work_item",
      pending: "stage-pip-coding",
    };
    const stateColor: Record<string, string> = {
      done: "bg-emerald-500",
      active: "bg-[var(--aria-primary)]",
      blocked: "bg-amber-500",
      pending: "bg-[var(--aria-line)]",
    };

    for (const [state, testId] of Object.entries(pipByState)) {
      const pip = screen.getByTestId(testId);
      expect(pip).toHaveClass("h-2", "w-2", "rounded-full");
      expect(pip).toHaveClass(stateColor[state]);
    }
  });

  it("4 个 pip 之间有 3 条 w-3 连接线", () => {
    render(<StageMiniGraph pips={queueRow().stagePips} />);

    const graph = screen.getByTestId("stage-mini-graph");
    const connectors = Array.from(graph.children).filter(
      (child) => !child.hasAttribute("data-state"),
    );
    expect(connectors).toHaveLength(3);
    for (const connector of connectors) {
      expect(connector).toHaveClass("w-3");
    }
  });
});

describe("IssueQueueRow", () => {
  it("单行 h-11 布局且标题 truncate 展示", () => {
    render(<IssueQueueRow row={queueRow()} focused={false} onSelect={vi.fn()} />);

    const row = screen.getByTestId("issue-queue-row");
    expect(row).toHaveClass("h-11");

    const title = screen.getByTestId("issue-queue-row-title");
    expect(title).toHaveClass("truncate");
    expect(title).toHaveTextContent("会话过期提示");
  });

  it("行内渲染 stage mini-graph", () => {
    render(<IssueQueueRow row={queueRow()} focused={false} onSelect={vi.fn()} />);

    const row = screen.getByTestId("issue-queue-row");
    const graph = screen.getByTestId("stage-mini-graph");
    expect(row).toContainElement(graph);
    expect(
      within(graph).getAllByTestId(/stage-pip-(story|design|work_item|coding)/),
    ).toHaveLength(4);
  });

  it("focused 时 aria-current=true 且带 3px 左侧主色条与面板背景", () => {
    render(<IssueQueueRow row={queueRow()} focused={true} onSelect={vi.fn()} />);

    const row = screen.getByTestId("issue-queue-row");
    expect(row).toHaveAttribute("aria-current", "true");
    expect(row).toHaveClass("border-l-[3px]");
    expect(row).toHaveClass("border-l-[var(--aria-primary)]");
    expect(row).toHaveClass("bg-[var(--aria-panel)]");
  });

  it("非 focused 时无 aria-current，左条透明且无面板背景", () => {
    render(<IssueQueueRow row={queueRow()} focused={false} onSelect={vi.fn()} />);

    const row = screen.getByTestId("issue-queue-row");
    expect(row.getAttribute("aria-current")).toBeNull();
    expect(row).toHaveClass("border-l-[3px]", "border-l-transparent");
    expect(row).not.toHaveClass("bg-[var(--aria-panel)]");
    expect(row).not.toHaveClass("border-l-[var(--aria-primary)]");
  });

  it("生成与删除按钮存在、默认 opacity-0 且 hover/focus-within 显现", () => {
    render(
      <IssueQueueRow
        row={queueRow()}
        focused={false}
        onSelect={vi.fn()}
        onGenerateStorySpec={vi.fn()}
        onDelete={vi.fn()}
      />,
    );

    const generate = screen.getByRole("button", { name: "生成 Story Spec" });
    expect(generate).toHaveClass("opacity-0");
    expect(generate).toHaveClass("group-hover:opacity-100");
    expect(generate).toHaveClass("group-focus-within:opacity-100");

    const remove = screen.getByRole("button", { name: /删除/ });
    expect(remove).toHaveClass("opacity-0");
    expect(remove).toHaveClass("group-hover:opacity-100");
    expect(remove).toHaveClass("group-focus-within:opacity-100");
  });

  it("未传回调时不渲染操作按钮", () => {
    render(<IssueQueueRow row={queueRow()} focused={false} onSelect={vi.fn()} />);

    expect(screen.queryByRole("button", { name: "生成 Story Spec" })).toBeNull();
    expect(screen.queryByRole("button", { name: /删除/ })).toBeNull();
  });

  it("deleting 时操作按钮禁用且行 aria-busy", () => {
    render(
      <IssueQueueRow
        row={queueRow()}
        focused={false}
        onSelect={vi.fn()}
        onGenerateStorySpec={vi.fn()}
        onDelete={vi.fn()}
        deleting={true}
      />,
    );

    expect(screen.getByTestId("issue-queue-row")).toHaveAttribute(
      "aria-busy",
      "true",
    );
    expect(screen.getByRole("button", { name: "生成 Story Spec" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /删除/ })).toBeDisabled();
  });

  it("点击标题触发 onSelect", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <IssueQueueRow row={queueRow()} focused={false} onSelect={onSelect} />,
    );

    await user.click(screen.getByTestId("issue-queue-row-title"));

    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it("点击生成/删除按钮分别触发回调且不触发 onSelect", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onGenerateStorySpec = vi.fn();
    const onDelete = vi.fn();
    render(
      <IssueQueueRow
        row={queueRow()}
        focused={false}
        onSelect={onSelect}
        onGenerateStorySpec={onGenerateStorySpec}
        onDelete={onDelete}
      />,
    );

    await user.click(screen.getByRole("button", { name: "生成 Story Spec" }));
    expect(onGenerateStorySpec).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /删除/ }));
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
  });
});
