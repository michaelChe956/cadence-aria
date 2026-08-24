import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { WorkItemRepositoryGroup } from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  IssueLifecycleDetail,
} from "./IssueLifecycleWorkbenchParts";
import type { WorkbenchStageKey } from "./StageStepper";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  workItemRecord,
  workItemRepositoryGroupRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

describe("IssueLifecycleWorkbenchParts work item repository grouping (REQ-TGT-05)", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("renders work items grouped by target repository with alias and status", async () => {
    const groups = [
      workItemRepositoryGroupRecord({
        target_repository_id: "repo_api",
        alias: "api",
        status: "pending",
        compatibility_projection: false,
        items: [
          workItemRecord({
            work_item_id: "work_item_backend",
            issue_id: "issue_0001",
            title: "后端 API 实现",
            kind: "backend",
            plan_status: "confirmed",
            execution_status: "pending",
          }),
        ],
      }),
      workItemRepositoryGroupRecord({
        target_repository_id: "repo_web",
        alias: "web",
        status: "completed",
        compatibility_projection: false,
        items: [
          workItemRecord({
            work_item_id: "work_item_frontend",
            issue_id: "issue_0001",
            title: "前端 UI 实现",
            kind: "frontend",
            plan_status: "confirmed",
            execution_status: "completed",
          }),
        ],
      }),
    ];
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ workItemRepositoryGroups: groups }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 7：队列改为 IssueQueue，行选择按钮名为「选择 Issue <标题>」。
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    // 每组标注仓库名（alias）。
    expect(workItemRegion).toHaveTextContent("api");
    expect(workItemRegion).toHaveTextContent("web");
    // 每组标注仓库级聚合状态。
    expect(workItemRegion).toHaveTextContent("pending");
    expect(workItemRegion).toHaveTextContent("completed");
    // 组内 Work Item 标题可见。
    expect(workItemRegion).toHaveTextContent("后端 API 实现");
    expect(workItemRegion).toHaveTextContent("前端 UI 实现");
  });

  it("marks the legacy unassigned group as compatibility projection", async () => {
    const groups = [
      workItemRepositoryGroupRecord({
        target_repository_id: null,
        alias: "未指定仓库",
        status: "blocked",
        compatibility_projection: true,
        items: [
          workItemRecord({
            work_item_id: "work_item_legacy",
            issue_id: "issue_0001",
            title: "遗留 Work Item",
            kind: "backend",
            plan_status: "confirmed",
            execution_status: "blocked",
          }),
        ],
      }),
    ];
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ workItemRepositoryGroups: groups }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    expect(workItemRegion).toHaveTextContent("未指定仓库");
    expect(workItemRegion).toHaveTextContent("blocked");
    expect(workItemRegion).toHaveTextContent("兼容投影");
    expect(workItemRegion).toHaveTextContent("遗留 Work Item");
  });

  it("keeps flat work item cards when work_item_repository_groups is empty (single repo compatibility)", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ workItemRepositoryGroups: [] }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    // 空分组（单仓/无分组）回退扁平展示：仍展示 Work Item Group 卡片。
    expect(workItemRegion).toHaveTextContent("Work Item Group");
  });
});

// Task 6：IssueLifecycleDetail 重构为阶段标签工作区的单测（直接渲染组件）。
function stageIssueCard(issueId: string): LifecycleCardData {
  return {
    kind: "issue",
    issueId,
    id: issueId,
    title: `Issue ${issueId}`,
    status: "draft",
    version: null,
    preview: "Issue 描述",
    sourceIds: [],
    raw: {},
  } as unknown as LifecycleCardData;
}

function stageSpecCard(
  kind: "story_spec" | "design_spec",
  issueId: string,
): LifecycleCardData {
  return {
    kind,
    issueId,
    id: `${kind}_0001`,
    title: kind === "story_spec" ? "会话过期提示" : "前端提示设计",
    status: "confirmed",
    version: 1,
    preview: null,
    sourceIds: [],
    artifactVersions: [],
    raw: {},
  } as unknown as LifecycleCardData;
}

function stageWorkItemGroupCard(issueId: string): LifecycleCardData {
  return {
    kind: "work_item_group",
    issueId,
    id: "issue_plan_0001",
    title: "Work Item Group",
    status: "draft",
    version: null,
    preview: null,
    sourceIds: [],
    childWorkItemIds: [],
    artifactVersions: [],
    raw: {},
  } as unknown as LifecycleCardData;
}

function renderDetail({
  issue = stageIssueCard("issue_0001"),
  storySpecs = [],
  designSpecs = [],
  workItems = [],
  workItemRepositoryGroups = [],
  onGenerateForStage = vi.fn(),
}: {
  issue?: LifecycleCardData | null;
  storySpecs?: LifecycleCardData[];
  designSpecs?: LifecycleCardData[];
  workItems?: LifecycleCardData[];
  workItemRepositoryGroups?: Parameters<
    typeof IssueLifecycleDetail
  >[0]["workItemRepositoryGroups"];
  onGenerateForStage?: (stage: WorkbenchStageKey) => void;
} = {}) {
  const callbacks = {
    onSelect: vi.fn(),
    onOpenFullIssue: vi.fn(),
    onDelete: vi.fn(),
    onGenerateForStage,
  };
  const view = render(
    <IssueLifecycleDetail
      issue={issue}
      storySpecs={storySpecs}
      designSpecs={designSpecs}
      workItems={workItems}
      workItemRepositoryGroups={workItemRepositoryGroups}
      selectedKey={null}
      deletingKey={null}
      {...callbacks}
    />,
  );
  return { view, callbacks };
}

describe("IssueLifecycleDetail 阶段标签工作区 (Task 6)", () => {
  it("有 story 无 design 时默认选中 design 阶段，且仅渲染当前阶段区域", async () => {
    const user = userEvent.setup();
    renderDetail({
      storySpecs: [stageSpecCard("story_spec", "issue_0001")],
      designSpecs: [],
      workItems: [],
    });

    // 默认阶段规则：story 有产物、design 空 -> 默认 design。
    expect(screen.getByTestId("stage-tab-design")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByTestId("stage-tab-story")).toHaveAttribute(
      "aria-selected",
      "false",
    );
    // 同一时刻只渲染当前阶段的区域。
    expect(
      screen.getByRole("region", { name: "Design Spec 内容" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Story Spec 内容" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Work Item 内容" }),
    ).not.toBeInTheDocument();
    // selected-issue-preview 仍在详情头部。
    expect(screen.getByTestId("selected-issue-preview")).toHaveTextContent(
      "Issue 描述",
    );
    // stepper 计数与 pip 状态：story done / design active / work_item pending。
    expect(screen.getByTestId("stage-tab-count-story")).toHaveTextContent("1");
    expect(screen.getByTestId("stage-tab-count-design")).toHaveTextContent(
      "0",
    );
    expect(screen.getByTestId("stage-tab-count-work_item")).toHaveTextContent(
      "0",
    );
    expect(screen.getByTestId("stage-tab-pip-story")).toHaveAttribute(
      "data-state",
      "done",
    );
    expect(screen.getByTestId("stage-tab-pip-design")).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.getByTestId("stage-tab-pip-work_item")).toHaveAttribute(
      "data-state",
      "pending",
    );

    await user.click(screen.getByTestId("stage-tab-story"));
    expect(screen.getByTestId("stage-tab-story")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.getByRole("region", { name: "Story Spec 内容" }),
    ).toHaveTextContent("会话过期提示");
    expect(
      screen.queryByRole("region", { name: "Design Spec 内容" }),
    ).not.toBeInTheDocument();
  });

  it("空 design 面板显示「暂无内容」与「生成 Design Spec」主按钮并触发回调", async () => {
    const onGenerateForStage = vi.fn();
    const user = userEvent.setup();
    renderDetail({
      storySpecs: [stageSpecCard("story_spec", "issue_0001")],
      designSpecs: [],
      onGenerateForStage,
    });

    const designRegion = screen.getByRole("region", {
      name: "Design Spec 内容",
    });
    expect(designRegion).toHaveTextContent("暂无内容");
    await user.click(
      within(designRegion).getByRole("button", { name: "生成 Design Spec" }),
    );
    expect(onGenerateForStage).toHaveBeenCalledWith("design");
  });

  it("story/work_item 空阶段主按钮分别回调对应阶段", async () => {
    const onGenerateForStage = vi.fn();
    const user = userEvent.setup();
    renderDetail({ onGenerateForStage });

    // 全空 -> 默认 story 阶段。
    const storyRegion = screen.getByRole("region", {
      name: "Story Spec 内容",
    });
    await user.click(
      within(storyRegion).getByRole("button", { name: "生成 Story Spec" }),
    );
    expect(onGenerateForStage).toHaveBeenCalledWith("story");

    await user.click(screen.getByTestId("stage-tab-work_item"));
    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    expect(workItemRegion).toHaveTextContent("暂无内容");
    await user.click(
      within(workItemRegion).getByRole("button", {
        name: "准备 Work Item Plan",
      }),
    );
    expect(onGenerateForStage).toHaveBeenCalledWith("work_item");
  });

  it("非空阶段不显示生成主按钮", () => {
    renderDetail({
      storySpecs: [stageSpecCard("story_spec", "issue_0001")],
      designSpecs: [stageSpecCard("design_spec", "issue_0001")],
      workItems: [stageWorkItemGroupCard("issue_0001")],
    });

    // 默认 work_item（story/design 均有产物）。
    expect(screen.getByTestId("stage-tab-work_item")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    expect(
      within(workItemRegion).queryByRole("button", {
        name: "准备 Work Item Plan",
      }),
    ).not.toBeInTheDocument();
    expect(workItemRegion).toHaveTextContent("Work Item Group");
  });

  it("切换 Issue 时重置为默认阶段，同一 Issue 数据刷新不重置", async () => {
    const user = userEvent.setup();
    const issueA = stageIssueCard("issue_0001");
    const issueB = stageIssueCard("issue_0002");
    const props = {
      storySpecs: [stageSpecCard("story_spec", "issue_0001")],
      designSpecs: [stageSpecCard("design_spec", "issue_0001")],
      workItems: [stageWorkItemGroupCard("issue_0001")],
    };
    const { view } = renderDetail({ issue: issueA, ...props });

    // 默认 work_item -> 手动切到 story。
    await user.click(screen.getByTestId("stage-tab-story"));
    expect(screen.getByTestId("stage-tab-story")).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // 同一 Issue 数据刷新（design 变化）不重置用户已选阶段。
    view.rerender(
      <IssueLifecycleDetail
        issue={issueA}
        {...props}
        workItemRepositoryGroups={[]}
        selectedKey={null}
        deletingKey={null}
        onSelect={vi.fn()}
        onOpenFullIssue={vi.fn()}
        onDelete={vi.fn()}
        onGenerateForStage={vi.fn()}
      />,
    );
    expect(screen.getByTestId("stage-tab-story")).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // 切换 Issue -> 重置为默认阶段（story+design 有产物 -> work_item）。
    view.rerender(
      <IssueLifecycleDetail
        issue={issueB}
        {...props}
        workItemRepositoryGroups={[]}
        selectedKey={null}
        deletingKey={null}
        onSelect={vi.fn()}
        onOpenFullIssue={vi.fn()}
        onDelete={vi.fn()}
        onGenerateForStage={vi.fn()}
      />,
    );
    expect(screen.getByTestId("stage-tab-work_item")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.queryByRole("region", { name: "Story Spec 内容" }),
    ).not.toBeInTheDocument();
  });

  it("work_item 阶段存在等待依赖且无产物时 pip 为 blocked，分组 testid 保留在阶段页内", async () => {
    const user = userEvent.setup();
    const dependentItem = workItemRecord({
      work_item_id: "work_item_backend",
      issue_id: "issue_0001",
      title: "后端 API",
      depends_on: ["work_item_frontend"],
    });
    const pendingItem = workItemRecord({
      work_item_id: "work_item_frontend",
      issue_id: "issue_0001",
      title: "前端 UI",
      execution_status: "pending",
    });
    renderDetail({
      storySpecs: [],
      designSpecs: [],
      workItems: [],
      workItemRepositoryGroups: [
        workItemRepositoryGroupRecord({
          target_repository_id: "repo_api",
          items: [dependentItem, pendingItem],
        }) as unknown as WorkItemRepositoryGroup,
      ],
    });

    // story/design 均 -> 默认 story；work_item 无产物但存在等待依赖 -> blocked。
    expect(screen.getByTestId("stage-tab-pip-story")).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.getByTestId("stage-tab-pip-work_item")).toHaveAttribute(
      "data-state",
      "blocked",
    );

    await user.click(screen.getByTestId("stage-tab-work_item"));
    expect(
      screen.getByTestId("work-item-repository-group-repo_api"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).toHaveTextContent("后端 API");
  });

  it("未选 Issue 时保持原有空态文案与结构", () => {
    renderDetail({ issue: null });

    const detail = screen.getByRole("region", {
      name: "Issue 生命周期详情",
    });
    expect(detail).toHaveTextContent("选择一个 Issue");
    expect(screen.queryByTestId("stage-stepper")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Story Spec 内容" }),
    ).not.toBeInTheDocument();
  });
});
