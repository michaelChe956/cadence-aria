// PR #92 三审 Important 回归（焦点可见性）：迁组焦点还原目前只在「迁入行仍被渲染」
// 时生效——findRowTitleButton 找不到标题按钮时直接放弃，焦点掉回 <body>。两种场景
// 找不到按钮：行迁入默认折叠的组（「已完成」折叠态不渲染 rows），或行落在组渲染
// 上限之外（每组 rows 截断）。契约：队列在此时请求父层让该 Issue 可见
// （onEnsureIssueVisible(issueId)，父层展开所在组并提升该组渲染上限），随后照常把
// 焦点还原到该行标题按钮；行确实无法可见时清除待还原记录，不抢焦点、不无限请求。
import { useState } from "react";
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

const UNLOADED_PIPS: IssueQueueRowData["stagePips"] = [];
const COMPLETED_PIPS: IssueQueueRowData["stagePips"] = [
  { stage: "story", state: "done" },
  { stage: "design", state: "done" },
  { stage: "work_item", state: "done" },
  { stage: "coding", state: "done" },
];

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

function groupHeader(key: IssueQueueGroupKey): HTMLElement {
  const header = screen
    .getAllByTestId("issue-queue-group-header")
    .find((node) => node.getAttribute("data-group-key") === key);
  if (header === undefined) {
    throw new Error(`group header not found: ${key}`);
  }
  return header;
}

function rowGroupKey(issueId: string): string | null {
  const row = screen
    .getAllByTestId("issue-queue-row")
    .find((node) => node.getAttribute("data-issue-id") === issueId);
  return (
    row
      ?.closest('[data-testid="issue-queue-group-rows"]')
      ?.getAttribute("data-group-key") ?? null
  );
}

// 扮演 workbench 的 harness：持有组折叠态与每组渲染上限（截断发生在父层重派生），
// 收到 onEnsureIssueVisible 时让目标 Issue 可见——展开其所在组并把该组上限提为
// 无限制（workbench 的等价动作：展开折叠组 + 以无上限 perGroupLimit 重派生该组）。
function FocusQueueHarness({
  fullGroups,
  initialCollapsedGroups = ["completed"],
  perGroupLimit = Number.MAX_SAFE_INTEGER,
  onEnsureIssueVisible,
}: {
  fullGroups: IssueQueueGroup[];
  initialCollapsedGroups?: IssueQueueGroupKey[];
  perGroupLimit?: number;
  onEnsureIssueVisible?: (issueId: string) => void;
}) {
  const [collapsedGroups, setCollapsedGroups] = useState<IssueQueueGroupKey[]>(
    initialCollapsedGroups,
  );
  const [raisedGroups, setRaisedGroups] = useState<
    ReadonlySet<IssueQueueGroupKey>
  >(() => new Set());
  const groups = fullGroups.map((group) =>
    raisedGroups.has(group.key)
      ? group
      : { ...group, rows: group.rows.slice(0, perGroupLimit) },
  );
  const handleEnsureIssueVisible = (issueId: string) => {
    onEnsureIssueVisible?.(issueId);
    const group = fullGroups.find((candidate) =>
      candidate.rows.some((row) => row.issueId === issueId),
    );
    if (!group) {
      return;
    }
    setCollapsedGroups((prev) =>
      prev.filter((key) => key !== group.key),
    );
    setRaisedGroups((prev) => {
      if (prev.has(group.key)) {
        return prev;
      }
      const next = new Set(prev);
      next.add(group.key);
      return next;
    });
  };
  return (
    <IssueQueue
      groups={groups}
      focusedIssueId={null}
      collapsedGroups={collapsedGroups}
      onToggleGroup={(key) =>
        setCollapsedGroups((prev) =>
          prev.includes(key)
            ? prev.filter((candidate) => candidate !== key)
            : [...prev, key],
        )
      }
      filterText=""
      onFilterTextChange={() => {}}
      onSelectIssue={() => {}}
      onGenerateStorySpec={() => {}}
      onDeleteIssue={() => {}}
      deletingIssueId={null}
      onEnsureIssueVisible={handleEnsureIssueVisible}
    />
  );
}

describe("IssueQueue 焦点可见性（迁入折叠组 / 超出组渲染上限）", () => {
  it("焦点行迁入默认折叠的「已完成」组：请求父层后组被展开、行被渲染、焦点还原到标题按钮", () => {
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
    const completedRow = queueRow({
      issueId: "issue_9003",
      title: "登录失败重试",
      group: "completed",
      stagePips: COMPLETED_PIPS,
    });

    const { rerender } = render(
      <FocusQueueHarness
        fullGroups={[
          queueGroup("unloaded", [moving, staying]),
          queueGroup("completed", [completedRow]),
        ]}
      />,
    );

    // 「已完成」组默认折叠：迁移目标组当前不渲染任何行。
    expect(groupHeader("completed")).toHaveAttribute("aria-expanded", "false");

    // 键盘焦点落在未加载行（unloaded 组）的标题按钮上。
    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 详情到达：同一 issue 迁入「已完成」组（折叠态不渲染 rows），unloaded 只剩 staying。
    rerender(
      <FocusQueueHarness
        fullGroups={[
          queueGroup("unloaded", [staying]),
          queueGroup("completed", [
            completedRow,
            {
              ...moving,
              group: "completed",
              stagePips: COMPLETED_PIPS,
            },
          ]),
        ]}
      />,
    );

    // 契约：队列请求父层让该 Issue 可见后，组被展开、行被渲染。
    expect(groupHeader("completed")).toHaveAttribute("aria-expanded", "true");
    expect(rowGroupKey("issue_9001")).toBe("completed");

    // 焦点还原到迁组行的新标题按钮，而不是掉回 <body>。
    const afterButton = rowTitleButton("issue_9001");
    expect(document.activeElement).not.toBe(document.body);
    expect(document.activeElement).toBe(afterButton);
  });

  it("焦点行迁入组渲染上限之外：请求父层提升该组上限后行被渲染、焦点还原到标题按钮", () => {
    const moving = queueRow({
      issueId: "issue_9001",
      title: "邀请链接过期",
      group: "unloaded",
      stagePips: UNLOADED_PIPS,
    });
    const codingFirst = queueRow({
      issueId: "issue_9004",
      title: "分支保护检查",
      group: "coding",
    });

    const { rerender } = render(
      <FocusQueueHarness
        fullGroups={[
          queueGroup("unloaded", [moving]),
          queueGroup("coding", [codingFirst]),
        ]}
        initialCollapsedGroups={[]}
        perGroupLimit={1}
      />,
    );

    // 上限 1：coding 组首行在渲染内、迁移目标位置（第 2 行）在渲染外。
    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 详情到达：行迁入 coding 组尾部（下标 1 ≥ 上限 1，父层截断后不渲染）。
    rerender(
      <FocusQueueHarness
        fullGroups={[
          queueGroup("unloaded", []),
          queueGroup("coding", [
            codingFirst,
            { ...moving, group: "coding" },
          ]),
        ]}
        initialCollapsedGroups={[]}
        perGroupLimit={1}
      />,
    );

    // 契约：请求父层后该组渲染上限被提升，行出现在组内。
    expect(rowGroupKey("issue_9001")).toBe("coding");

    // 焦点还原到迁组行的新标题按钮，而不是掉回 <body>。
    const afterButton = rowTitleButton("issue_9001");
    expect(document.activeElement).not.toBe(document.body);
    expect(document.activeElement).toBe(afterButton);
  });

  it("行确实无法可见（如被过滤移除）：只请求一次，清除待还原记录且不抢焦点", () => {
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
    const onEnsureIssueVisible = vi.fn();

    const { rerender } = render(
      <FocusQueueHarness
        fullGroups={[queueGroup("unloaded", [removed, survivor])]}
        initialCollapsedGroups={[]}
        onEnsureIssueVisible={onEnsureIssueVisible}
      />,
    );

    const beforeButton = rowTitleButton("issue_9001");
    beforeButton.focus();
    expect(document.activeElement).toBe(beforeButton);

    // 该行被过滤移除，任何组都不含该 issue：父层无能为力。
    rerender(
      <FocusQueueHarness
        fullGroups={[queueGroup("unloaded", [survivor])]}
        initialCollapsedGroups={[]}
        onEnsureIssueVisible={onEnsureIssueVisible}
      />,
    );

    expect(onEnsureIssueVisible).toHaveBeenCalledTimes(1);
    expect(onEnsureIssueVisible).toHaveBeenCalledWith("issue_9001");
    // 不抢焦点：焦点掉回 <body> 后保持，不被强行移到其他行。
    expect(document.activeElement).toBe(document.body);

    // 后续无关重渲染不再重复请求，也不把焦点抢到幸存行上。
    rerender(
      <FocusQueueHarness
        fullGroups={[
          queueGroup("unloaded", [
            { ...survivor, title: "审计日志接入（更新）" },
          ]),
        ]}
        initialCollapsedGroups={[]}
        onEnsureIssueVisible={onEnsureIssueVisible}
      />,
    );

    expect(onEnsureIssueVisible).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(document.body);
  });
});
