// 焦点回归（IssueQueue）：行从「生命周期待查看」组迁入阶段组（详情到达触发父层重派生）
// 时，行会换父容器 —— React 无法跨父复用 DOM 节点，旧行节点被销毁后键盘焦点掉回
// <body>。契约：焦点必须显式还原到迁组行自己的标题按钮上；DOM 结构、region 名
// 「Issue 卡片列表」、aria-current 语义保持不变。
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  IssueQueueGroup,
  IssueQueueGroupKey,
  IssueQueueRowData,
} from "./issue-queue-derivation";
import { IssueQueue } from "./IssueQueue";

function queueRow(overrides: Partial<IssueQueueRowData> = {}): IssueQueueRowData {
  return {
    issueId: "issue_0001",
    title: "会话过期提示",
    status: "draft",
    stagePips: [
      { stage: "story", state: "done" },
      { stage: "design", state: "done" },
      { stage: "work_item", state: "done" },
      { stage: "coding", state: "active" },
    ],
    group: "coding",
    storyCount: 1,
    designCount: 1,
    workItemCount: 1,
    ...overrides,
  };
}

function queueGroup(
  key: IssueQueueGroupKey,
  rows: IssueQueueRowData[],
  total = rows.length,
): IssueQueueGroup {
  return { key, rows, total };
}

function queueProps(
  groups: IssueQueueGroup[],
  overrides: { focusedIssueId?: string | null } = {},
) {
  return {
    groups,
    focusedIssueId: overrides.focusedIssueId ?? null,
    collapsedGroups: [],
    onToggleGroup: vi.fn(),
    filterText: "",
    onFilterTextChange: vi.fn(),
    onSelectIssue: vi.fn(),
    onGenerateStorySpec: vi.fn(),
    onDeleteIssue: vi.fn(),
    deletingIssueId: null,
  };
}

// 行标题按钮 = data-issue-id 命中行内的 issue-queue-row-title 所在 button。
function rowTitleButton(issueId: string): HTMLButtonElement {
  const row = screen
    .getAllByTestId("issue-queue-row")
    .find((node) => node.getAttribute("data-issue-id") === issueId);
  if (row === undefined) {
    throw new Error(`issue-queue-row not found: ${issueId}`);
  }
  const title = within(row).getByTestId("issue-queue-row-title");
  const button = title.closest("button");
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`title button not found: ${issueId}`);
  }
  return button;
}

// 未加载行（unloaded 组）无 pips；详情到达后按 needs_story 派生规则给 active story pip。
const UNLOADED_PIPS: IssueQueueRowData["stagePips"] = [];
const NEEDS_STORY_PIPS: IssueQueueRowData["stagePips"] = [
  { stage: "story", state: "active" },
  { stage: "design", state: "pending" },
  { stage: "work_item", state: "pending" },
  { stage: "coding", state: "pending" },
];

describe("IssueQueue 焦点保持", () => {
  it("行从「生命周期待查看」迁入阶段组后，键盘焦点仍停留在该行标题按钮上", () => {
    const moving = queueRow({
      issueId: "issue_9001",
      title: "邀请链接过期",
      group: "unloaded",
      stagePips: UNLOADED_PIPS,
    });
    const staying = queueRow({
      issueId: "issue_9002",
      title: "审计日志接入",
      group: "unloaded",
      stagePips: UNLOADED_PIPS,
    });
    const existing = queueRow({
      issueId: "issue_9003",
      title: "登录失败重试",
      group: "needs_story",
      stagePips: NEEDS_STORY_PIPS,
    });

    const { rerender } = render(
      <IssueQueue
        {...queueProps([
          queueGroup("unloaded", [moving, staying]),
          queueGroup("needs_story", [existing]),
        ])}
      />,
    );

    // 键盘焦点落在迁组行的标题按钮上。
    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 详情到达：同一 issue 迁入 needs_story 组，unloaded 组只剩 staying；
    // 同时该行成为当前选中行（aria-current 语义随迁组保持）。
    rerender(
      <IssueQueue
        {...queueProps(
          [
            queueGroup("unloaded", [staying]),
            queueGroup("needs_story", [
              existing,
              {
                ...moving,
                group: "needs_story",
                stagePips: NEEDS_STORY_PIPS,
              },
            ]),
          ],
          { focusedIssueId: "issue_9001" },
        )}
      />,
    );

    // 契约：焦点在迁组行的新标题按钮上，而不是 <body>。
    const afterButton = rowTitleButton("issue_9001");
    expect(document.activeElement).not.toBe(document.body);
    expect(document.activeElement).toBe(afterButton);

    // 迁组确实发生（新按钮位于 needs_story 组内），region 名与 aria-current 保持。
    expect(
      afterButton.closest('[data-testid="issue-queue-group-rows"][data-group-key="needs_story"]'),
    ).not.toBeNull();
    expect(
      screen.getByRole("region", { name: "Issue 卡片列表" }),
    ).toBeInTheDocument();
    expect(afterButton.closest('[data-testid="issue-queue-row"]')).toHaveAttribute(
      "aria-current",
      "true",
    );
  });

  it("未迁组的普通重渲染不重建行节点，焦点保持且不被抢走", () => {
    const target = queueRow({
      issueId: "issue_9001",
      title: "邀请链接过期",
      group: "coding",
    });
    const groupsBefore = [queueGroup("coding", [target])];
    const groupsAfter = [
      queueGroup("coding", [target, queueRow({ issueId: "issue_9002", title: "新增行" })]),
    ];

    const { rerender } = render(
      <IssueQueue {...queueProps(groupsBefore)} />,
    );
    const button = rowTitleButton("issue_9001");
    button.focus();
    expect(document.activeElement).toBe(button);

    rerender(<IssueQueue {...queueProps(groupsAfter)} />);

    expect(document.activeElement).toBe(button);
    expect(document.activeElement).toBe(rowTitleButton("issue_9001"));
  });

  it("焦点所在行被整体移除（如被过滤）时不强行恢复焦点到其他行", () => {
    const removed = queueRow({
      issueId: "issue_9001",
      title: "邀请链接过期",
      group: "unloaded",
      stagePips: UNLOADED_PIPS,
    });
    const survivor = queueRow({
      issueId: "issue_9002",
      title: "审计日志接入",
      group: "unloaded",
      stagePips: UNLOADED_PIPS,
    });

    const { rerender } = render(
      <IssueQueue
        {...queueProps([queueGroup("unloaded", [removed, survivor])])}
      />,
    );
    const button = rowTitleButton("issue_9001");
    button.focus();
    expect(document.activeElement).toBe(button);

    // 该行被过滤掉，unloaded 组只剩另一行。
    rerender(
      <IssueQueue
        {...queueProps([queueGroup("unloaded", [survivor])])}
      />,
    );

    expect(document.activeElement).toBe(document.body);
    expect(document.activeElement).not.toBe(rowTitleButton("issue_9002"));
  });
});
