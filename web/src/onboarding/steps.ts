/**
 * onboarding 步骤集中配置（沿 whats-new/changelog.ts 集中配置先例）。
 *
 * 本模块只承载「引导如何解释与定位界面」：顺序、标题、说明、示意资源、稳定
 * `data-testid` 锚点与启用条件。它不承载业务 JSX，也不驱动任何业务状态推进。
 */

export type OnboardingVisual =
  | { kind: "screenshot"; src: string; alt: string }
  | { kind: "diagram"; alt: string };

export type OnboardingStepGate = {
  /** 所属 feature 标识；用于配置化覆盖启用开关。 */
  feature: string;
  /** 本期默认启用状态；自动化模式选择步骤本期为 false。 */
  enabled: boolean;
  /** late-bind：目标锚点就绪才纳入，默认未就绪即视为不可用。 */
  lateBind?: () => boolean;
};

export type OnboardingStep = {
  id: string;
  order: number;
  title: string;
  description: string;
  visual: OnboardingVisual;
  anchorTestId: string;
  gate?: OnboardingStepGate;
};

export type OnboardingFilterContext = {
  /** 覆盖 step.gate.enabled 的 feature 开关（P1 仅翻转配置即可启用）。 */
  features?: Record<string, boolean>;
};

export interface OnboardingProgress {
  current: number;
  total: number;
}

export const AUTOMATION_MODE_FEATURE = "onboarding.automation-mode-selection";
export const AUTOMATION_MODE_STEP_ID = "automation-mode-selection";
export const AUTOMATION_MODE_ANCHOR = "onboarding-anchor-automation-mode";

export const ONBOARDING_STEPS: readonly OnboardingStep[] = [
  {
    id: AUTOMATION_MODE_STEP_ID,
    order: 0,
    title: "选择运行模式",
    description:
      "工作台支持手动与自动化两种运行模式：自动化可交由系统推进阶段。本期引导先覆盖手动模式的完整路径。",
    visual: { kind: "diagram", alt: "运行模式选择入口示意" },
    anchorTestId: AUTOMATION_MODE_ANCHOR,
    gate: {
      feature: AUTOMATION_MODE_FEATURE,
      // 本期默认关闭：P1 提供稳定自动化模式选择锚点后，仅翻转此配置即可纳入，
      // 渲染器无需新增分支。
      enabled: false,
      lateBind: () =>
        typeof document !== "undefined" &&
        document.querySelector(`[data-testid="${AUTOMATION_MODE_ANCHOR}"]`) !== null,
    },
  },
  {
    id: "create-project",
    order: 1,
    title: "添加 Project",
    description:
      "在左侧 Projects 区点击「新建 Project」，创建流程容器；代码库、Issue 与后续阶段都挂在它下面。",
    visual: { kind: "diagram", alt: "Project 侧栏新建按钮位置示意" },
    anchorTestId: "onboarding-anchor-project-create",
  },
  {
    id: "add-single-repository",
    order: 2,
    title: "添加代码库（仅单库）",
    description:
      "选中 Project 后点击「添加代码库」，绑定一个物理代码库。本期引导只覆盖单库（单仓）路径；逻辑代码库可稍后按需组合。",
    visual: { kind: "diagram", alt: "代码库添加按钮与单仓路径示意" },
    anchorTestId: "onboarding-anchor-codebase-add",
  },
  {
    id: "create-issue",
    order: 3,
    title: "创建 Issue（含基准分支）",
    description:
      "点击头部「新建 Issue」，填写标题、绑定代码库，并选择基准分支。基准分支在创建时锁定，之后修改基线需要新建 Issue。",
    visual: { kind: "diagram", alt: "新建 Issue 与基准分支选择示意" },
    anchorTestId: "onboarding-anchor-issue-create",
  },
  {
    id: "generate-story",
    order: 4,
    title: "生成 Story",
    description:
      "选中 Issue 后进入 Story 阶段，生成 Story Spec，作为后续设计与实现的唯一需求来源。",
    visual: { kind: "diagram", alt: "Stage 步进器 Story 阶段示意" },
    anchorTestId: "onboarding-anchor-stage-story",
  },
  {
    id: "confirm-story",
    order: 5,
    title: "确认 Story",
    description:
      "检查 Story 内容无误后确认。确认动作在 Workspace 会话页完成：从 Story 卡片抽屉点「打开 Workspace」，在产物面板点「确认定稿」。",
    visual: { kind: "diagram", alt: "Story 内容区与确认入口示意" },
    anchorTestId: "onboarding-anchor-story-confirm",
  },
  {
    id: "generate-design",
    order: 6,
    title: "生成 Design",
    description: "基于已确认的 Story 进入 Design 阶段，生成 Design Spec。",
    visual: { kind: "diagram", alt: "Stage 步进器 Design 阶段示意" },
    anchorTestId: "onboarding-anchor-stage-design",
  },
  {
    id: "confirm-design",
    order: 7,
    title: "确认 Design",
    description:
      "检查 Design 内容无误后确认，同样经「打开 Workspace」在会话页点「确认定稿」完成。",
    visual: { kind: "diagram", alt: "Design 内容区与确认入口示意" },
    anchorTestId: "onboarding-anchor-design-confirm",
  },
  {
    id: "generate-work-item-plan",
    order: 8,
    title: "生成 Work Item Plan",
    description:
      "进入 Work Item 阶段生成本次实现的 Work Item Plan，它会把工作拆成可执行单元。",
    visual: { kind: "diagram", alt: "Stage 步进器 Work Item 阶段示意" },
    anchorTestId: "onboarding-anchor-stage-work-item-plan",
  },
  {
    id: "confirm-work-item-plan-review",
    order: 9,
    title: "确认 Work Item Plan · 第一道门",
    description:
      "先通过第一道计划审阅门：核对计划与契约缺口，确认计划内容本身可以送审 / 审阅通过。这是两道门中的第一道。",
    visual: { kind: "diagram", alt: "计划审阅区域示意" },
    anchorTestId: "onboarding-anchor-plan-review-gate",
  },
  {
    id: "confirm-work-item-plan-final",
    order: 10,
    title: "确认 Work Item Plan · 第二道门",
    description:
      "再通过第二道计划最终确认门：这是进入 Coding 前的最终确认，与第一道门是两个独立的确认门，不要合并理解。",
    visual: { kind: "diagram", alt: "计划最终确认区域示意" },
    anchorTestId: "onboarding-anchor-plan-final-gate",
  },
  {
    id: "enter-coding-workspace",
    order: 11,
    title: "进入 Coding Workspace",
    description:
      "选中已确认的 Work Item，打开右侧抽屉并点击「进入 Coding Workspace」。Coding 是独立页面，引导只提示入口、不会自动跳转。",
    visual: { kind: "diagram", alt: "抽屉中的进入 Coding 入口示意" },
    anchorTestId: "drawer-open-coding-workspace",
  },
  {
    id: "start-coding",
    order: 12,
    title: "启动 Coding",
    description:
      "在 Coding Workspace 中显式点击「开始 Coding」启动本轮执行；运行中可随时中止。",
    visual: { kind: "diagram", alt: "开始 Coding 按钮示意" },
    anchorTestId: "onboarding-anchor-coding-start",
  },
  {
    id: "coding-progress-complete",
    order: 13,
    title: "查看进度并确认完成",
    description:
      "在顶部组进度区查看各执行单元进度；全部完成后按页面提示在 Coding Workspace 确认完成。",
    visual: { kind: "diagram", alt: "Coding 组进度区示意" },
    anchorTestId: "onboarding-anchor-coding-progress",
  },
];

/**
 * 先按 gate 的 `enabled`（可被 context.features 覆盖）与可选 `lateBind` 过滤，
 * 再按 `order` 排序；返回新数组，不修改入参。
 */
export function filterOnboardingSteps(
  steps: readonly OnboardingStep[],
  context: OnboardingFilterContext = {},
): OnboardingStep[] {
  return steps
    .filter((step) => {
      const gate = step.gate;
      if (!gate) {
        return true;
      }
      const enabled = context.features?.[gate.feature] ?? gate.enabled;
      if (!enabled) {
        return false;
      }
      if (gate.lateBind && !gate.lateBind()) {
        return false;
      }
      return true;
    })
    .slice()
    .sort((left, right) => left.order - right.order);
}

export function getEnabledOnboardingSteps(
  context: OnboardingFilterContext = {},
): OnboardingStep[] {
  return filterOnboardingSteps(ONBOARDING_STEPS, context);
}

/** 过滤后列表的进度换算（越界与空列表安全）。 */
export function onboardingProgress(index: number, total: number): OnboardingProgress {
  const safeTotal = Math.max(total, 0);
  if (safeTotal === 0) {
    return { current: 0, total: 0 };
  }
  const clamped = Math.min(Math.max(index, 0), safeTotal - 1);
  return { current: clamped + 1, total: safeTotal };
}
