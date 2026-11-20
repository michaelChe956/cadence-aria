// 焦点还原意图（IssueQueue）：迁组还原只应修复「迁组本身造成的焦点丢失」。
// 用户主动把焦点移出队列（如点击非聚焦空白区域使焦点落回 <body>，或聚焦队列外
// 元素）后，行的迁组重渲染不得把键盘焦点抢回行标题按钮；焦点仍在行内时迁组，
// 必须还原到新组内该行标题按钮（原始缺陷回归守卫）。
// DOM 结构、region 名「Issue 卡片列表」、aria-current 语义保持不变。
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

describe("IssueQueue 焦点还原意图", () => {
  it("用户先把焦点移出队列（落回 <body>）后行迁组，不把焦点抢回行标题按钮", () => {
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

    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 用户点击非聚焦空白区域：焦点主动离开队列，落回 <body>。
    beforeButton.blur();
    expect(document.activeElement).toBe(document.body);

    // 详情到达：该 issue 迁入 needs_story 组。
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

    // 契约：用户已主动离开队列，迁组不得抢回焦点（activeElement 保持 <body>）。
    const afterButton = rowTitleButton("issue_9001");
    expect(document.activeElement).toBe(document.body);
    expect(document.activeElement).not.toBe(afterButton);
  });

  it("用户先把焦点移到队列外元素后行迁组，不把焦点抢回行标题按钮", () => {
    const moving = queueRow({
      issueId: "issue_9001",
      title: "邀请链接过期",
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
      <div>
        <button type="button" data-testid="outside-workbench-button">
          队列外按钮
        </button>
        <IssueQueue
          {...queueProps([
            queueGroup("unloaded", [moving]),
            queueGroup("needs_story", [existing]),
          ])}
        />
      </div>,
    );

    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 用户聚焦队列外的其他可交互元素（workbench 外层控件）。
    const outsideButton = screen.getByTestId("outside-workbench-button");
    outsideButton.focus();
    expect(document.activeElement).toBe(outsideButton);

    // 详情到达：该 issue 迁入 needs_story 组。
    rerender(
      <div>
        <button type="button" data-testid="outside-workbench-button">
          队列外按钮
        </button>
        <IssueQueue
          {...queueProps(
            [
              queueGroup("unloaded", []),
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
        />
      </div>,
    );

    // 契约：焦点仍归用户指定的队列外元素，迁组不得抢占。
    expect(document.activeElement).toBe(outsideButton);
    expect(document.activeElement).not.toBe(rowTitleButton("issue_9001"));
  });

  it("焦点仍在行标题按钮上时行迁组，焦点还原到新组内该行标题按钮", () => {
    const moving = queueRow({
      issueId: "issue_9001",
      title: "邀请链接过期",
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
          queueGroup("unloaded", [moving]),
          queueGroup("needs_story", [existing]),
        ])}
      />,
    );

    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 详情到达：同一 issue 迁入 needs_story 组（焦点从未被用户移走）。
    rerender(
      <IssueQueue
        {...queueProps(
          [
            queueGroup("unloaded", []),
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

    // 契约（原始缺陷回归）：焦点还原到迁组行在新组内的标题按钮。
    const afterButton = rowTitleButton("issue_9001");
    expect(document.activeElement).not.toBe(document.body);
    expect(document.activeElement).toBe(afterButton);
    expect(
      afterButton.closest(
        '[data-testid="issue-queue-group-rows"][data-group-key="needs_story"]',
      ),
    ).not.toBeNull();
    expect(
      screen.getByRole("region", { name: "Issue 卡片列表" }),
    ).toBeInTheDocument();
    expect(afterButton.closest('[data-testid="issue-queue-row"]')).toHaveAttribute(
      "aria-current",
      "true",
    );
  });
});
