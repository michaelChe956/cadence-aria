import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbenchHeader } from "../components/lifecycle/IssueLifecycleWorkbenchHeader";
import { ProjectSidebar } from "../components/lifecycle/ProjectSidebar";
import { StageStepper, type StageStepperStage } from "../components/lifecycle/StageStepper";
import { ActionButtons } from "../pages/CodingWorkspaceControls";
import { CodingWorkspaceGroupProgress } from "../pages/CodingWorkspaceGroupProgress";
import {
  AUTOMATION_MODE_ANCHOR,
  ONBOARDING_STEPS,
  getEnabledOnboardingSteps,
} from "./steps";

const stageFixtures: StageStepperStage[] = [
  { key: "story", label: "Story", count: 0, state: "active" },
  { key: "design", label: "Design", count: 0, state: "pending" },
  { key: "work_item", label: "Work Item", count: 0, state: "pending" },
];

describe("onboarding 锚点契约（宿主组件渲染）", () => {
  it("ProjectSidebar 暴露新建 Project 与添加代码库锚点", () => {
    render(
      <ProjectSidebar
        projects={[]}
        codebases={[]}
        repositories={[]}
        selectedProjectId={null}
        issueCount={0}
        busy={false}
        onSelectProject={() => {}}
        onCreateProject={() => {}}
        onAddCodebase={() => {}}
        onDeleteProject={() => {}}
        onDeleteRepository={() => {}}
        onDeleteLogicalCodebase={() => {}}
      />,
    );
    expect(screen.getByTestId("onboarding-anchor-project-create")).toHaveTextContent("新建 Project");
    expect(screen.getByTestId("onboarding-anchor-codebase-add")).toHaveTextContent("添加代码库");
  });

  it("工作台头部暴露新建 Issue 锚点", () => {
    render(
      <IssueLifecycleWorkbenchHeader
        projectName="演示项目"
        focusedIssueId={null}
        canCreateIssue
        onShowAll={() => {}}
        onRefresh={() => {}}
        onCreateIssue={() => {}}
      />,
    );
    expect(screen.getByTestId("onboarding-anchor-issue-create")).toHaveTextContent("新建 Issue");
  });

  it("阶段步进器暴露三个阶段入口锚点", () => {
    render(<StageStepper stages={stageFixtures} activeStage="story" onSelect={() => {}} />);
    expect(screen.getByTestId("onboarding-anchor-stage-story")).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-anchor-stage-design")).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-anchor-stage-work-item-plan")).toBeInTheDocument();
  });

  it("Coding 控件暴露启动 Coding 锚点", () => {
    render(
      <ActionButtons
        api={{ startCoding: vi.fn(), restartCoding: vi.fn(), abortAttempt: vi.fn(), finalConfirm: vi.fn() } as never}
        stage="prepare_context"
        status={null}
      />,
    );
    expect(screen.getByTestId("onboarding-anchor-coding-start")).toHaveTextContent("开始 Coding");
  });

  it("Coding 组进度区暴露进度/完成确认锚点", () => {
    render(
      <CodingWorkspaceGroupProgress
        planId="plan_0001"
        currentWorkItemId="wi_0001"
        units={[{ logical_work_item_id: "wi_0001", status: "completed" } as never]}
      />,
    );
    expect(screen.getByTestId("onboarding-anchor-coding-progress")).toBeInTheDocument();
  });

  it("配置引用的锚点全部为稳定非空字符串，且手动 13 步互不重复（复用的 Coding 入口除外）", () => {
    const steps = getEnabledOnboardingSteps();
    const anchors = steps.map((step) => step.anchorTestId);
    expect(anchors.every((anchor) => typeof anchor === "string" && anchor.length > 0)).toBe(true);
    const duplicated = anchors.filter((anchor, index) => anchors.indexOf(anchor) !== index);
    expect(duplicated).toEqual([]);
    expect(ONBOARDING_STEPS.length).toBeGreaterThanOrEqual(14);
    expect(AUTOMATION_MODE_ANCHOR.length).toBeGreaterThan(0);
  });
});
