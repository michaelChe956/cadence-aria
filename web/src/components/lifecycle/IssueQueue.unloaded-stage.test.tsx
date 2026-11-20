// OpenSpec workbench-progressive-issue-loading「详情未到时的行」契约：
// unloaded 组行仍渲染四阶段 mini-graph（全部 unknown 态，语义上区别于 pending）、
// 「删除 Issue」入口可用、「生成 Story Spec」入口不可用；既有四种状态的渲染不变。
// unloaded 组由真实的 deriveIssueQueue(pendingIssues) 派生，锁定派生层与容器接线两层行为。
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProductIssue } from "../../api/types";
import type {
  IssueQueueGroup,
  IssueQueueRowData,
} from "./issue-queue-derivation";
import { deriveIssueQueue } from "./issue-queue-derivation";
import { IssueQueue } from "./IssueQueue";

const STAGE_ORDER = ["story", "design", "work_item", "coding"] as const;

function pendingIssue(overrides: Partial<ProductIssue> = {}): ProductIssue {
  return {
    issue_id: "issue_8001",
    project_id: "project_0001",
    repo_id: null,
    base_branch: null,
    workspace_id: null,
    task_id: null,
    session_id: null,
    title: "会话过期提示",
    description: null,
    change_id: "change_0001",
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

// 已加载行 fixture：四个 pip 各占一种既有状态，锁定 loaded 行渲染不被本次改动改变。
function loadedRow(overrides: Partial<IssueQueueRowData> = {}): IssueQueueRowData {
  return {
    issueId: "issue_8002",
    title: "登录失败重试",
    status: "in_progress",
    stagePips: [
      { stage: "story", state: "done" },
      { stage: "design", state: "active" },
      { stage: "work_item", state: "blocked" },
      { stage: "coding", state: "pending" },
    ],
    group: "coding",
    storyCount: 1,
    designCount: 1,
    workItemCount: 1,
    ...overrides,
  };
}

function rowByIssueId(issueId: string): HTMLElement {
  const row = screen
    .getAllByTestId("issue-queue-row")
    .find((node) => node.getAttribute("data-issue-id") === issueId);
  if (row === undefined) {
    throw new Error(`issue-queue-row not found: ${issueId}`);
  }
  return row;
}

function renderQueue(groups: IssueQueueGroup[]) {
  const props = {
    groups,
    focusedIssueId: null,
    collapsedGroups: [],
    onToggleGroup: vi.fn(),
    filterText: "",
    onFilterTextChange: vi.fn(),
    onSelectIssue: vi.fn(),
    onGenerateStorySpec: vi.fn(),
    onDeleteIssue: vi.fn(),
    deletingIssueId: null,
  };
  render(<IssueQueue {...props} />);
  return props;
}

describe("deriveIssueQueue 未加载行 stagePips", () => {
  it("pendingIssues 派生的 unloaded 行生成四个阶段全 unknown 的 pips", () => {
    const groups = deriveIssueQueue([], {
      pendingIssues: [pendingIssue()],
    });

    expect(groups).toHaveLength(1);
    expect(groups[0]?.key).toBe("unloaded");
    const row = groups[0]?.rows[0];
    expect(row?.stagePips).toHaveLength(4);
    expect(row?.stagePips.map((pip) => `${pip.stage}:${pip.state}`)).toEqual([
      "story:unknown",
      "design:unknown",
      "work_item:unknown",
      "coding:unknown",
    ]);
  });
});

describe("IssueQueue 未加载行（unloaded 组）", () => {
  function unloadedGroups(): IssueQueueGroup[] {
    // unloaded 组走真实派生层；loaded 组手搭以锁定既有状态渲染。
    return [
      ...deriveIssueQueue([], { pendingIssues: [pendingIssue()] }),
      {
        key: "coding",
        rows: [loadedRow()],
        total: 1,
      } satisfies IssueQueueGroup,
    ];
  }

  it("unloaded 行渲染四阶段 mini-graph，四个 pip 均 data-state=unknown 且提示为「未知」", () => {
    renderQueue(unloadedGroups());

    const row = rowByIssueId("issue_8001");
    expect(within(row).getByTestId("stage-mini-graph")).toBeInTheDocument();

    for (const stage of STAGE_ORDER) {
      const pip = within(row).getByTestId(`stage-pip-${stage}`);
      expect(pip).toHaveAttribute("data-state", "unknown");
      // 未知态提示不得呈现为「待开始」（语义不同）。
      expect(pip).toHaveAttribute("title", expect.stringContaining("未知"));
      expect(pip).not.toHaveAttribute("title", expect.stringContaining("待开始"));
    }
  });

  it("unloaded 行的删除入口存在且可用，点击触发 onDeleteIssue(issueId)", async () => {
    const user = userEvent.setup();
    const props = renderQueue(unloadedGroups());

    const row = rowByIssueId("issue_8001");
    const deleteButton = within(row).getByRole("button", {
      name: "删除 Issue 会话过期提示",
    });
    expect(deleteButton).toBeEnabled();

    await user.click(deleteButton);
    expect(props.onDeleteIssue).toHaveBeenCalledTimes(1);
    expect(props.onDeleteIssue).toHaveBeenCalledWith("issue_8001");
  });

  it("unloaded 行不渲染「生成 Story Spec」入口", () => {
    renderQueue(unloadedGroups());

    const row = rowByIssueId("issue_8001");
    expect(
      within(row).queryByRole("button", { name: /生成 Story Spec/ }),
    ).toBeNull();
  });

  it("已加载行的四种既有状态 data-state 渲染不变", () => {
    renderQueue(unloadedGroups());

    const row = rowByIssueId("issue_8002");
    const expectedStates: Record<(typeof STAGE_ORDER)[number], string> = {
      story: "done",
      design: "active",
      work_item: "blocked",
      coding: "pending",
    };
    for (const stage of STAGE_ORDER) {
      expect(
        within(row).getByTestId(`stage-pip-${stage}`),
      ).toHaveAttribute("data-state", expectedStates[stage]);
    }
  });
});
