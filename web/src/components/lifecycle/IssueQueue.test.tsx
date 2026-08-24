// Task 4（容器组件）：IssueQueue 吸顶过滤条 + 分组折叠 + 显示更多的渲染单测。
// 契约与 Plan 逐字一致：外层 region 名 `Issue 卡片列表`、组按 ISSUE_QUEUE_GROUP_ORDER、
// 折叠态不渲染 rows、rows.length<total 时「显示更多（+N）」且点击记录本地 state。
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type {
  IssueQueueGroup,
  IssueQueueGroupKey,
  IssueQueueRowData,
} from "./issue-queue-derivation";
import { ISSUE_QUEUE_GROUP_ORDER } from "./issue-queue-derivation";
import { IssueQueue } from "./IssueQueue";

// 组名中文映射（Plan 逐字）。
const GROUP_LABEL: Record<IssueQueueGroupKey, string> = {
  needs_story: "待生成 Story",
  needs_design: "待生成 Design",
  needs_work_item: "待拆 Work Item",
  blocked: "阻塞",
  coding: "编码中",
  completed: "已完成",
};

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

// 默认 fixture：六个组各一例；coding 组 2 行 / total 5 用于「显示更多」。
// 总数 chip = 各组 total 之和 = 1+1+1+1+5+1 = 10。
function defaultGroups(): IssueQueueGroup[] {
  return [
    queueGroup("needs_story", [
      queueRow({ issueId: "issue_0001", title: "会话过期提示", group: "needs_story" }),
    ]),
    queueGroup("needs_design", [
      queueRow({ issueId: "issue_0002", title: "登录态刷新", group: "needs_design" }),
    ]),
    queueGroup("needs_work_item", [
      queueRow({ issueId: "issue_0003", title: "权限缓存清理", group: "needs_work_item" }),
    ]),
    queueGroup("blocked", [
      queueRow({ issueId: "issue_0004", title: "审计日志接入", group: "blocked" }),
    ]),
    queueGroup(
      "coding",
      [
        queueRow({ issueId: "issue_0005", title: "登录失败重试", group: "coding" }),
        queueRow({ issueId: "issue_0006", title: "邀请链接过期", group: "coding" }),
      ],
      5,
    ),
    queueGroup("completed", [
      queueRow({ issueId: "issue_0007", title: "旧接口下线", group: "completed" }),
    ]),
  ];
}

function renderQueue(
  overrides: {
    groups?: IssueQueueGroup[];
    focusedIssueId?: string | null;
    collapsedGroups?: IssueQueueGroupKey[];
    filterText?: string;
    deletingIssueId?: string | null;
    onToggleGroup?: ReturnType<typeof vi.fn>;
    onFilterTextChange?: ReturnType<typeof vi.fn>;
    onSelectIssue?: ReturnType<typeof vi.fn>;
    onGenerateStorySpec?: ReturnType<typeof vi.fn>;
    onDeleteIssue?: ReturnType<typeof vi.fn>;
    onShowMoreGroup?: ReturnType<typeof vi.fn>;
  } = {},
) {
  const props = {
    groups: overrides.groups ?? defaultGroups(),
    focusedIssueId: overrides.focusedIssueId ?? null,
    collapsedGroups: overrides.collapsedGroups ?? [],
    onToggleGroup: overrides.onToggleGroup ?? vi.fn(),
    filterText: overrides.filterText ?? "",
    onFilterTextChange: overrides.onFilterTextChange ?? vi.fn(),
    onSelectIssue: overrides.onSelectIssue ?? vi.fn(),
    onGenerateStorySpec: overrides.onGenerateStorySpec ?? vi.fn(),
    onDeleteIssue: overrides.onDeleteIssue ?? vi.fn(),
    deletingIssueId: overrides.deletingIssueId ?? null,
    ...(overrides.onShowMoreGroup
      ? { onShowMoreGroup: overrides.onShowMoreGroup }
      : {}),
  };
  render(<IssueQueue {...props} />);
  return props;
}

function findByGroupKey(
  testId: string,
  key: IssueQueueGroupKey,
): HTMLElement {
  const matched = screen
    .getAllByTestId(testId)
    .find((node) => node.getAttribute("data-group-key") === key);
  if (matched === undefined) {
    throw new Error(`element not found: ${testId} [${key}]`);
  }
  return matched;
}

function groupSection(key: IssueQueueGroupKey): HTMLElement {
  return findByGroupKey("issue-queue-group", key);
}

function groupHeader(key: IssueQueueGroupKey): HTMLElement {
  return findByGroupKey("issue-queue-group-header", key);
}

describe("IssueQueue 结构", () => {
  it("外层为名为 Issue 卡片列表 的 region，头部含 Issues 标题、总数 chip 与过滤输入", () => {
    renderQueue();

    const region = screen.getByRole("region", { name: "Issue 卡片列表" });
    expect(region).toHaveClass("flex", "min-h-0", "flex-col");

    expect(
      within(region).getByRole("heading", { name: "Issues" }),
    ).toBeInTheDocument();

    // 总数 chip = 各组 total 之和（1+1+1+1+5+1）。
    const chip = within(region).getByTestId("issue-queue-total-chip");
    expect(chip).toHaveTextContent("10");

    const input = within(region).getByLabelText("过滤 Issues");
    expect(input).toHaveValue("");
  });

  it("组列表容器 overflow-y-auto，头部吸顶", () => {
    renderQueue();

    const list = screen.getByTestId("issue-queue-group-list");
    expect(list).toHaveClass("overflow-y-auto");

    const header = screen.getByTestId("issue-queue-header");
    expect(header).toHaveClass("sticky", "top-0");
  });

  it("六个组按 ISSUE_QUEUE_GROUP_ORDER 渲染并使用中文组名", () => {
    const shuffled = [...defaultGroups()].reverse();
    renderQueue({ groups: shuffled });

    const list = screen.getByTestId("issue-queue-group-list");
    const keys = Array.from(
      list.querySelectorAll('[data-testid="issue-queue-group"]'),
    ).map((node) => node.getAttribute("data-group-key"));
    expect(keys).toEqual(ISSUE_QUEUE_GROUP_ORDER);

    for (const key of ISSUE_QUEUE_GROUP_ORDER) {
      expect(groupHeader(key)).toHaveTextContent(GROUP_LABEL[key]);
    }
  });

  it("空分组时渲染空态提示且总数 chip 为 0", () => {
    renderQueue({ groups: [] });

    const region = screen.getByRole("region", { name: "Issue 卡片列表" });
    expect(region).toHaveTextContent("没有匹配的 Issue。");
    expect(screen.getByTestId("issue-queue-total-chip")).toHaveTextContent("0");
    expect(screen.queryAllByTestId("issue-queue-group")).toHaveLength(0);
  });
});

describe("IssueQueue 分组折叠", () => {
  it("组头显示 rows/total 计数并触发 onToggleGroup(key)", async () => {
    const user = userEvent.setup();
    const onToggleGroup = vi.fn();
    renderQueue({ onToggleGroup });

    // coding 组 2 行 / total 5。
    expect(groupHeader("coding")).toHaveTextContent("2/5");
    expect(groupHeader("needs_story")).toHaveTextContent("1/1");

    await user.click(groupHeader("blocked"));
    expect(onToggleGroup).toHaveBeenCalledTimes(1);
    expect(onToggleGroup).toHaveBeenCalledWith("blocked");
  });

  it("collapsedGroups 中的组头 aria-expanded=false 且不渲染 rows", () => {
    renderQueue({ collapsedGroups: ["completed"] });

    expect(groupHeader("completed")).toHaveAttribute("aria-expanded", "false");
    expect(groupHeader("coding")).toHaveAttribute("aria-expanded", "true");

    const completed = groupSection("completed");
    expect(
      within(completed).queryAllByTestId("issue-queue-row"),
    ).toHaveLength(0);

    // 展开组仍渲染 rows。
    const coding = groupSection("coding");
    expect(within(coding).getAllByTestId("issue-queue-row")).toHaveLength(2);
  });
});

describe("IssueQueue 过滤", () => {
  it("过滤输入受控展示 filterText 并逐字触发 onFilterTextChange", async () => {
    const user = userEvent.setup();
    const onFilterTextChange = vi.fn();
    // 受控输入需要父级真实回写 state，否则 React 每次击键后会还原 DOM 值。
    function FilterHarness() {
      const [text, setText] = useState("登录");
      return (
        <IssueQueue
          groups={defaultGroups()}
          focusedIssueId={null}
          collapsedGroups={[]}
          onToggleGroup={vi.fn()}
          filterText={text}
          onFilterTextChange={(value) => {
            setText(value);
            onFilterTextChange(value);
          }}
          onSelectIssue={vi.fn()}
          onGenerateStorySpec={vi.fn()}
          onDeleteIssue={vi.fn()}
        />
      );
    }
    render(<FilterHarness />);

    const input = screen.getByLabelText("过滤 Issues");
    expect(input).toHaveValue("登录");

    await user.type(input, "失效");
    expect(onFilterTextChange).toHaveBeenCalledWith("登录失");
    expect(onFilterTextChange).toHaveBeenLastCalledWith("登录失效");
    expect(input).toHaveValue("登录失效");
  });
});

describe("IssueQueue 显示更多", () => {
  it("rows.length < total 的组渲染「显示更多（+N）」，足额组不渲染", () => {
    renderQueue();

    const showMore = within(groupSection("coding")).getByTestId(
      "issue-queue-show-more",
    );
    expect(showMore).toHaveTextContent("显示更多（+3）");
    expect(showMore).toHaveAttribute("data-group-key", "coding");

    // 足额组（rows.length === total）没有该按钮。
    expect(
      within(groupSection("needs_story")).queryByTestId("issue-queue-show-more"),
    ).toBeNull();
  });

  it("点击「显示更多」记录到本地 state：该组按钮消失，其他组不受影响", async () => {
    const user = userEvent.setup();
    const groups = defaultGroups();
    // blocked 组也构造截断：1 行 / total 3。
    groups[3] = queueGroup(
      "blocked",
      [queueRow({ issueId: "issue_0004", title: "审计日志接入", group: "blocked" })],
      3,
    );
    renderQueue({ groups });

    await user.click(
      within(groupSection("coding")).getByTestId("issue-queue-show-more"),
    );

    expect(
      within(groupSection("coding")).queryByTestId("issue-queue-show-more"),
    ).toBeNull();
    // 其他截断组的按钮仍在。
    expect(
      within(groupSection("blocked")).getByTestId("issue-queue-show-more"),
    ).toBeInTheDocument();
  });

  it("折叠组不渲染「显示更多」按钮", () => {
    renderQueue({ collapsedGroups: ["coding"] });

    expect(screen.queryByTestId("issue-queue-show-more")).toBeNull();
    expect(groupHeader("coding")).toHaveTextContent("2/5");
  });

  // Task 4 评审遗留 Important-1：受控模式下「显示更多」必须交给父层真正追加渲染。
  it("传入 onShowMoreGroup 时点击回调组 key，且不靠本地 state 隐藏入口", async () => {
    const user = userEvent.setup();
    const onShowMoreGroup = vi.fn();
    renderQueue({ onShowMoreGroup });

    await user.click(
      within(groupSection("coding")).getByTestId("issue-queue-show-more"),
    );

    expect(onShowMoreGroup).toHaveBeenCalledTimes(1);
    expect(onShowMoreGroup).toHaveBeenCalledWith("coding");
    // 受控模式：父层未重派生前入口仍在（隐藏由 rows.length === total 决定，而非本地 state）。
    expect(
      within(groupSection("coding")).getByTestId("issue-queue-show-more"),
    ).toBeInTheDocument();
  });

  it("受控模式下父层重派生补齐 rows 后入口自然消失", async () => {
    const user = userEvent.setup();
    // 父层 harness：命中组用更高 perGroupLimit 的结果（rows 补齐到 total）替换。
    function ShowMoreHarness() {
      const [expanded, setExpanded] = useState<IssueQueueGroupKey[]>([]);
      const groups = defaultGroups().map((group) =>
        expanded.includes(group.key)
          ? queueGroup(
              group.key,
              [
                ...group.rows,
                queueRow({ issueId: "issue_0008", group: group.key }),
                queueRow({ issueId: "issue_0009", group: group.key }),
                queueRow({ issueId: "issue_0010", group: group.key }),
              ],
              group.total,
            )
          : group,
      );
      return (
        <IssueQueue
          groups={groups}
          focusedIssueId={null}
          collapsedGroups={[]}
          onToggleGroup={vi.fn()}
          filterText=""
          onFilterTextChange={vi.fn()}
          onSelectIssue={vi.fn()}
          onGenerateStorySpec={vi.fn()}
          onDeleteIssue={vi.fn()}
          onShowMoreGroup={(key) => setExpanded((prev) => [...prev, key])}
        />
      );
    }
    render(<ShowMoreHarness />);

    expect(
      within(groupSection("coding")).getAllByTestId("issue-queue-row"),
    ).toHaveLength(2);

    await user.click(
      within(groupSection("coding")).getByTestId("issue-queue-show-more"),
    );

    expect(
      within(groupSection("coding")).getAllByTestId("issue-queue-row"),
    ).toHaveLength(5);
    expect(groupHeader("coding")).toHaveTextContent("5/5");
    expect(
      within(groupSection("coding")).queryByTestId("issue-queue-show-more"),
    ).toBeNull();
  });

  // Task 4 评审遗留 Important-2：非受控模式下本地「已追加」state 必须随过滤文本复位。
  it("非受控模式下 filterText 变化复位本地已追加 state", async () => {
    const user = userEvent.setup();
    function FilterResetHarness() {
      const [text, setText] = useState("");
      return (
        <IssueQueue
          groups={defaultGroups()}
          focusedIssueId={null}
          collapsedGroups={[]}
          onToggleGroup={vi.fn()}
          filterText={text}
          onFilterTextChange={setText}
          onSelectIssue={vi.fn()}
          onGenerateStorySpec={vi.fn()}
          onDeleteIssue={vi.fn()}
        />
      );
    }
    render(<FilterResetHarness />);

    await user.click(
      within(groupSection("coding")).getByTestId("issue-queue-show-more"),
    );
    expect(
      within(groupSection("coding")).queryByTestId("issue-queue-show-more"),
    ).toBeNull();

    // 过滤文本变化 -> 组已重派生，本地已追加标记必须复位，入口重新出现。
    await user.type(screen.getByLabelText("过滤 Issues"), "登录");

    expect(
      within(groupSection("coding")).getByTestId("issue-queue-show-more"),
    ).toHaveTextContent("显示更多（+3）");
  });
});

describe("IssueQueue 行接线", () => {
  it("focusedIssueId 对应行 aria-current=true，其余行不是", () => {
    renderQueue({ focusedIssueId: "issue_0005" });

    const rows = screen.getAllByTestId("issue-queue-row");
    const focused = rows.find(
      (row) => row.getAttribute("data-issue-id") === "issue_0005",
    );
    expect(focused).toHaveAttribute("aria-current", "true");

    const others = rows.filter(
      (row) => row.getAttribute("data-issue-id") !== "issue_0005",
    );
    expect(others.length).toBeGreaterThan(0);
    for (const row of others) {
      expect(row.getAttribute("aria-current")).toBeNull();
    }
  });

  it("点击行标题触发 onSelectIssue(issueId)", async () => {
    const user = userEvent.setup();
    const onSelectIssue = vi.fn();
    renderQueue({ onSelectIssue });

    await user.click(
      screen.getByRole("button", { name: "选择 Issue 登录失败重试" }),
    );
    expect(onSelectIssue).toHaveBeenCalledTimes(1);
    expect(onSelectIssue).toHaveBeenCalledWith("issue_0005");
  });

  it("生成与删除按钮分别触发对应回调并携带 issueId", async () => {
    const user = userEvent.setup();
    const onGenerateStorySpec = vi.fn();
    const onDeleteIssue = vi.fn();
    renderQueue({ onGenerateStorySpec, onDeleteIssue });

    await user.click(
      within(groupSection("needs_story")).getByRole("button", {
        name: "生成 Story Spec",
      }),
    );
    expect(onGenerateStorySpec).toHaveBeenCalledTimes(1);
    expect(onGenerateStorySpec).toHaveBeenCalledWith("issue_0001");

    await user.click(
      within(groupSection("needs_story")).getByRole("button", {
        name: "删除 Issue 会话过期提示",
      }),
    );
    expect(onDeleteIssue).toHaveBeenCalledTimes(1);
    expect(onDeleteIssue).toHaveBeenCalledWith("issue_0001");
  });

  it("deletingIssueId 对应行 aria-busy=true", () => {
    renderQueue({ deletingIssueId: "issue_0001" });

    const rows = screen.getAllByTestId("issue-queue-row");
    const deleting = rows.find(
      (row) => row.getAttribute("data-issue-id") === "issue_0001",
    );
    expect(deleting).toHaveAttribute("aria-busy", "true");
  });
});
