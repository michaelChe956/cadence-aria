// Task 5（展示组件）：StageStepper 阶段步进器的渲染单测。
// 契约与 Plan 逐字一致：容器 role=tablist、每段 role=tab + aria-selected 仅 active 为 true、
// 计数 chip + pip 状态色点 + 段间连接线；点击/键盘触发 onSelect(key)；
// 视觉契约：hover 仅改颜色/边框/阴影不位移、150–300ms 过渡 + motion-reduce 降级、
// cursor-pointer、focus-visible 主色 ring。
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { StagePipState } from "./issue-queue-derivation";
import {
  StageStepper,
  type StageStepperStage,
  type WorkbenchStageKey,
} from "./StageStepper";

const STAGE_ORDER: WorkbenchStageKey[] = ["story", "design", "work_item"];

// 覆盖三种 pip 状态的 fixture：story done / design active / work_item pending。
function stageFixtures(): StageStepperStage[] {
  return [
    { key: "story", label: "Story", count: 2, state: "done" },
    { key: "design", label: "Design", count: 1, state: "active" },
    { key: "work_item", label: "Work Item", count: 0, state: "pending" },
  ];
}

// 与 StageMiniGraph 相同的 pip 状态色映射（Plan 逐字一致）。
const STATE_COLOR: Record<StagePipState, string> = {
  done: "bg-emerald-500",
  active: "bg-[var(--aria-primary)]",
  blocked: "bg-amber-500",
  pending: "bg-[var(--aria-line)]",
};

// 视觉规范允许的 hover 反馈前缀：仅颜色/边框/阴影（不得有位移/尺寸类）。
const ALLOWED_HOVER_PREFIXES = [
  "hover:bg-",
  "hover:text-",
  "hover:border-",
  "hover:shadow-",
];

describe("StageStepper", () => {
  it("容器为 role=tablist，渲染三个 role=tab 段并按传入顺序展示标签", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    const stepper = screen.getByTestId("stage-stepper");
    expect(stepper).toHaveAttribute("role", "tablist");
    expect(stepper).toHaveAttribute("aria-label", "生命周期阶段");

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(3);
    expect(tabs.map((tab) => tab.getAttribute("data-testid"))).toEqual([
      "stage-tab-story",
      "stage-tab-design",
      "stage-tab-work_item",
    ]);

    expect(screen.getByTestId("stage-tab-story")).toHaveTextContent("Story");
    expect(screen.getByTestId("stage-tab-design")).toHaveTextContent("Design");
    expect(screen.getByTestId("stage-tab-work_item")).toHaveTextContent(
      "Work Item",
    );
  });

  it("每段渲染胶囊计数 chip，数值与 stages.count 一致", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="story"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByTestId("stage-tab-count-story")).toHaveTextContent("2");
    expect(screen.getByTestId("stage-tab-count-design")).toHaveTextContent("1");
    expect(screen.getByTestId("stage-tab-count-work_item")).toHaveTextContent(
      "0",
    );

    for (const key of STAGE_ORDER) {
      const chip = screen.getByTestId(`stage-tab-count-${key}`);
      expect(chip).toHaveClass("rounded-full");
      expect(within(chip).getByText(/^\d+$/)).toBeTruthy();
    }
  });

  it("aria-selected 仅 active 段为 true，其余段为 false", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByTestId("stage-tab-story")).toHaveAttribute(
      "aria-selected",
      "false",
    );
    expect(screen.getByTestId("stage-tab-design")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByTestId("stage-tab-work_item")).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("每段内 pip 色点 data-state 与状态色映射正确", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    const stateByKey: Record<WorkbenchStageKey, StagePipState> = {
      story: "done",
      design: "active",
      work_item: "pending",
    };

    for (const key of STAGE_ORDER) {
      const pip = within(screen.getByTestId(`stage-tab-${key}`)).getByTestId(
        `stage-tab-pip-${key}`,
      );
      expect(pip).toHaveAttribute("data-state", stateByKey[key]);
      expect(pip).toHaveClass("h-2", "w-2", "rounded-full");
      expect(pip).toHaveClass(STATE_COLOR[stateByKey[key]]);
    }
  });

  it("三段之间渲染 2 条连接线且对辅助技术隐藏", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    const stepper = screen.getByTestId("stage-stepper");
    const connectors = within(stepper).getAllByTestId(
      "stage-stepper-connector",
    );
    expect(connectors).toHaveLength(2);
    for (const connector of connectors) {
      expect(connector).toHaveAttribute("aria-hidden", "true");
      expect(connector).toHaveClass("rounded-full", "bg-[var(--aria-line)]");
    }
  });

  it("点击各段触发 onSelect 并携带对应 key", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="story"
        onSelect={onSelect}
      />,
    );

    await user.click(screen.getByTestId("stage-tab-design"));
    expect(onSelect).toHaveBeenCalledWith("design");

    await user.click(screen.getByTestId("stage-tab-work_item"));
    expect(onSelect).toHaveBeenCalledWith("work_item");

    await user.click(screen.getByTestId("stage-tab-story"));
    expect(onSelect).toHaveBeenCalledWith("story");

    expect(onSelect).toHaveBeenCalledTimes(3);
  });

  it("键盘 Enter/Space 激活当前聚焦段并触发 onSelect", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="story"
        onSelect={onSelect}
      />,
    );

    screen.getByTestId("stage-tab-design").focus();
    await user.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledWith("design");

    screen.getByTestId("stage-tab-work_item").focus();
    await user.keyboard(" ");
    expect(onSelect).toHaveBeenCalledWith("work_item");
  });

  it("视觉契约：cursor-pointer、200ms 过渡、motion-reduce 降级、focus-visible 主色 ring", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    for (const key of STAGE_ORDER) {
      const tab = screen.getByTestId(`stage-tab-${key}`);
      expect(tab).toHaveClass("cursor-pointer");
      expect(tab).toHaveClass("transition-colors", "duration-200");
      expect(tab).toHaveClass("motion-reduce:transition-none");
      expect(tab).toHaveClass(
        "focus-visible:outline-none",
        "focus-visible:ring-2",
        "focus-visible:ring-[var(--aria-primary)]",
      );
    }
  });

  it("hover 反馈仅允许颜色/边框/阴影类，不得有位移或尺寸类", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    for (const key of STAGE_ORDER) {
      const hoverClasses = Array.from(
        screen.getByTestId(`stage-tab-${key}`).classList,
      ).filter((cls) => cls.startsWith("hover:"));
      // 非 active 段必须有 hover 可发现性反馈；active 段保持选中态样式不被 hover 覆盖。
      if (key !== "design") {
        expect(hoverClasses.length).toBeGreaterThan(0);
      }
      for (const cls of hoverClasses) {
        expect(
          ALLOWED_HOVER_PREFIXES.some((prefix) => cls.startsWith(prefix)),
          `unexpected hover class: ${cls}`,
        ).toBe(true);
      }
    }
  });

  it("active 段用主色描边与主色软底，非 active 段用 line 描边 + muted 文本", () => {
    render(
      <StageStepper
        stages={stageFixtures()}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );

    const active = screen.getByTestId("stage-tab-design");
    expect(active).toHaveClass(
      "border-[var(--aria-primary)]",
      "bg-[var(--aria-primary-soft)]",
      "text-[var(--aria-primary)]",
    );

    const inactive = screen.getByTestId("stage-tab-story");
    expect(inactive).toHaveClass(
      "border-[var(--aria-line)]",
      "bg-[var(--aria-panel)]",
      "text-[var(--aria-ink-muted)]",
    );
    expect(inactive).not.toHaveClass("bg-[var(--aria-primary-soft)]");
  });
});

describe("StageStepper tab 与面板关联", () => {
  it("每个 tab 通过 aria-controls 指向对应面板 id", () => {
    render(
      <StageStepper
        stages={[
          { key: "story", label: "Story", count: 1, state: "done" },
          { key: "design", label: "Design", count: 0, state: "active" },
          { key: "work_item", label: "Work Item", count: 0, state: "pending" },
        ]}
        activeStage="design"
        onSelect={vi.fn()}
      />,
    );
    for (const key of ["story", "design", "work_item"] as const) {
      expect(screen.getByTestId(`stage-tab-${key}`)).toHaveAttribute(
        "aria-controls",
        `stage-panel-${key}`,
      );
    }
  });
});
