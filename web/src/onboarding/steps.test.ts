import { afterEach, describe, expect, it } from "vitest";
import {
  AUTOMATION_MODE_ANCHOR,
  AUTOMATION_MODE_FEATURE,
  AUTOMATION_MODE_STEP_ID,
  ONBOARDING_STEPS,
  filterOnboardingSteps,
  getEnabledOnboardingSteps,
  onboardingProgress,
  type OnboardingStep,
} from "./steps";

/** 新增第一步：Provider 管理（机器级能力核验）。 */
const PROVIDER_STEP_ID = "manage-providers";
const PROVIDER_STEP_ANCHOR = "provider-management-open";

/** 既有 13 步的 id 序列（逐字保留：新增步骤不得改动既有步骤的顺序与编号）。 */
const EXISTING_IDS = [
  "create-project",
  "add-logical-codebase",
  "create-issue",
  "generate-story",
  "confirm-story",
  "generate-design",
  "confirm-design",
  "generate-work-item-plan",
  "confirm-work-item-plan-review",
  "confirm-work-item-plan-final",
  "enter-coding-workspace",
  "start-coding",
  "coding-progress-complete",
];

/** 既有 13 步的 order 序列（逐字保留：新增步骤只能零位移插入，不得重排既有编号）。 */
const EXISTING_ORDERS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

const EXPECTED_IDS = [PROVIDER_STEP_ID, ...EXISTING_IDS];

const EXPECTED_ANCHORS = [
  PROVIDER_STEP_ANCHOR,
  "onboarding-anchor-project-create",
  "onboarding-anchor-codebase-add",
  "onboarding-anchor-issue-create",
  "onboarding-anchor-stage-story",
  "onboarding-anchor-story-confirm",
  "onboarding-anchor-stage-design",
  "onboarding-anchor-design-confirm",
  "onboarding-anchor-stage-work-item-plan",
  "onboarding-anchor-plan-review-gate",
  "onboarding-anchor-plan-final-gate",
  "drawer-open-coding-workspace",
  "onboarding-anchor-coding-start",
  "onboarding-anchor-coding-progress",
];

describe("onboarding 步骤配置", () => {
  it("手动模式提供 14 个按顺序启用的步骤：Provider 管理是第一步", () => {
    const steps = getEnabledOnboardingSteps();
    expect(steps).toHaveLength(14);
    expect(steps.map((step) => step.id)).toEqual(EXPECTED_IDS);
    expect(steps.map((step) => step.order)).toEqual([0, ...EXISTING_ORDERS]);
  });

  it("第一步是 Provider 管理步，锚点为 provider-management-open", () => {
    const steps = getEnabledOnboardingSteps();
    const first = steps[0];
    expect(first?.id).toBe(PROVIDER_STEP_ID);
    expect(first?.order).toBe(0);
    expect(first?.anchorTestId).toBe(PROVIDER_STEP_ANCHOR);
    // 文案必须讲清「为什么第一步」：先核验机器级 provider 能力，否则后续生成失败。
    expect(first?.title).toContain("Provider");
    expect(first?.description).toContain("provider");
    expect(first?.description).toContain("核验");
    expect(first?.visual.kind).toBe("diagram");
  });

  it("既有 13 步的 id 与 order 序列逐字未变（新增步骤零位移插入）", () => {
    const steps = getEnabledOnboardingSteps();
    const existing = steps.filter((step) => step.id !== PROVIDER_STEP_ID);
    expect(existing.map((step) => step.id)).toEqual(EXISTING_IDS);
    expect(existing.map((step) => step.order)).toEqual(EXISTING_ORDERS);
  });

  it("每个步骤都带标题、说明与锚点契约", () => {
    for (const step of getEnabledOnboardingSteps()) {
      expect(step.title.length).toBeGreaterThan(0);
      expect(step.description.length).toBeGreaterThan(0);
      expect(step.visual.alt.length).toBeGreaterThan(0);
      expect(step.anchorTestId.length).toBeGreaterThan(0);
    }
  });

  it("引导文案描述多仓流程：不再出现「单仓」路径，含「建逻辑代码库」步骤", () => {
    const steps = getEnabledOnboardingSteps();
    const codebaseStep = steps.find((step) => step.id === "add-logical-codebase");
    expect(codebaseStep).toBeDefined();
    expect(codebaseStep?.title).toContain("逻辑代码库");
    expect(codebaseStep?.description).toContain("聚合根");
    expect(codebaseStep?.description).toContain("登记成员");
    expect(codebaseStep?.visual.alt).not.toContain("单仓");
    const allCopy = steps
      .map((step) => `${step.title}\n${step.description}\n${step.visual.alt}`)
      .join("\n");
    expect(allCopy).not.toContain("单仓");
  });

  it("锚点映射与设计表一致，且计划两道门为不同锚点", () => {
    const steps = getEnabledOnboardingSteps();
    expect(steps.map((step) => step.anchorTestId)).toEqual(EXPECTED_ANCHORS);

    const reviewGate = steps.find((step) => step.id === "confirm-work-item-plan-review");
    const finalGate = steps.find((step) => step.id === "confirm-work-item-plan-final");
    expect(reviewGate?.anchorTestId).toBe("onboarding-anchor-plan-review-gate");
    expect(finalGate?.anchorTestId).toBe("onboarding-anchor-plan-final-gate");
    expect(reviewGate?.anchorTestId).not.toBe(finalGate?.anchorTestId);
  });

  it("自动化模式选择步骤默认禁用、被过滤且不占进度", () => {
    const automation = ONBOARDING_STEPS.find((step) => step.id === AUTOMATION_MODE_STEP_ID);
    expect(automation).toBeDefined();
    expect(automation?.gate?.feature).toBe(AUTOMATION_MODE_FEATURE);
    expect(automation?.gate?.enabled).toBe(false);
    expect(automation?.anchorTestId).toBe(AUTOMATION_MODE_ANCHOR);

    const steps = getEnabledOnboardingSteps();
    expect(steps.some((step) => step.id === AUTOMATION_MODE_STEP_ID)).toBe(false);
    expect(steps).toHaveLength(14);
    // 其余手动步骤顺序不因禁用自动化步骤而改变。
    expect(steps.map((step) => step.id)).toEqual(EXPECTED_IDS);
  });

  describe("late-bind：P1 稳定锚点可用后仅通过启用条件纳入", () => {
    afterEach(() => {
      document.body.innerHTML = "";
    });

    it("锚点缺失时即使 feature 开启也被过滤", () => {
      const steps = getEnabledOnboardingSteps({ features: { [AUTOMATION_MODE_FEATURE]: true } });
      expect(steps).toHaveLength(14);
      expect(steps.some((step) => step.id === AUTOMATION_MODE_STEP_ID)).toBe(false);
    });

    it("提供稳定锚点且 feature 开启后该步骤自动纳入，无需渲染器分支", () => {
      const anchor = document.createElement("div");
      anchor.setAttribute("data-testid", AUTOMATION_MODE_ANCHOR);
      document.body.appendChild(anchor);

      const steps = getEnabledOnboardingSteps({ features: { [AUTOMATION_MODE_FEATURE]: true } });
      expect(steps).toHaveLength(15);
      // Provider 管理恒为第 0 项（与同为 order 0 的自动化步骤并存时不抢位）。
      expect(steps[0]?.id).toBe(PROVIDER_STEP_ID);
      expect(steps[1]?.id).toBe(AUTOMATION_MODE_STEP_ID);
      expect(steps.slice(2).map((step) => step.id)).toEqual(EXISTING_IDS);
    });
  });

  it("filterOnboardingSteps 先过滤再按 order 排序，且不修改入参", () => {
    const synthetic: OnboardingStep[] = [
      { id: "b", order: 2, title: "B", description: "b", visual: { kind: "diagram", alt: "b" }, anchorTestId: "x-b" },
      { id: "a", order: 1, title: "A", description: "a", visual: { kind: "diagram", alt: "a" }, anchorTestId: "x-a" },
      {
        id: "g",
        order: 3,
        title: "G",
        description: "g",
        visual: { kind: "diagram", alt: "g" },
        anchorTestId: "x-g",
        gate: { feature: "demo", enabled: false },
      },
    ];
    const output = filterOnboardingSteps(synthetic);
    expect(output).not.toBe(synthetic);
    expect(output.map((step) => step.id)).toEqual(["a", "b"]);
    expect(synthetic.map((step) => step.id)).toEqual(["b", "a", "g"]);
  });

  describe("过滤后进度计算", () => {
    it("按过滤后的总数换算当前步序", () => {
      expect(onboardingProgress(0, 13)).toEqual({ current: 1, total: 13 });
      expect(onboardingProgress(12, 13)).toEqual({ current: 13, total: 13 });
    });

    it("越界与空列表安全", () => {
      expect(onboardingProgress(-3, 13)).toEqual({ current: 1, total: 13 });
      expect(onboardingProgress(99, 13)).toEqual({ current: 13, total: 13 });
      expect(onboardingProgress(0, 0)).toEqual({ current: 0, total: 0 });
    });
  });
});
