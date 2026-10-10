import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OnboardingWizard } from "./OnboardingWizard";
import type { OnboardingStep } from "./steps";

const steps: OnboardingStep[] = [
  {
    id: "create-project",
    order: 1,
    title: "添加 Project",
    description: "先创建一个 Project 作为流程容器。",
    visual: { kind: "diagram", alt: "新建 Project 按钮示意" },
    anchorTestId: "onboarding-anchor-project-create",
  },
  {
    id: "create-issue",
    order: 2,
    title: "创建 Issue",
    description: "填写标题、代码库与基准分支。",
    visual: { kind: "diagram", alt: "新建 Issue 按钮示意" },
    anchorTestId: "onboarding-anchor-issue-create",
  },
];

function renderWizard(index: number, overrides: Partial<ComponentProps<typeof OnboardingWizard>> = {}) {
  return render(
    <OnboardingWizard
      steps={steps}
      index={index}
      onPrev={vi.fn()}
      onNext={vi.fn()}
      onSkip={vi.fn()}
      onClose={vi.fn()}
      {...overrides}
    />,
  );
}

describe("OnboardingWizard", () => {
  afterEach(() => {
    // 先卸载 React 树（portal 节点由 React 从 body 摘除），再清手动附加的锚点。
    cleanup();
    document.body.innerHTML = "";
  });

  it("展示当前步骤标题、说明与过滤后的进度位置", () => {
    renderWizard(0);
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-step-title")).toHaveTextContent("添加 Project");
    expect(screen.getByTestId("onboarding-step-description")).toHaveTextContent("先创建一个 Project");
    expect(screen.getByTestId("onboarding-progress")).toHaveTextContent("第 1 / 共 2 步");
  });

  it("第一步禁用上一步，中间步骤可前进/后退", async () => {
    const user = userEvent.setup();
    const onNext = vi.fn();
    renderWizard(0, { onNext });
    expect(screen.getByTestId("onboarding-prev")).toBeDisabled();
    await user.click(screen.getByTestId("onboarding-next"));
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it("最后一步以完成收束并回传关闭", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    renderWizard(1, { onClose });
    expect(screen.queryByTestId("onboarding-next")).not.toBeInTheDocument();
    await user.click(screen.getByTestId("onboarding-finish"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("跳过在任一步骤可用", async () => {
    const user = userEvent.setup();
    const onSkip = vi.fn();
    renderWizard(0, { onSkip });
    await user.click(screen.getByTestId("onboarding-skip"));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });

  it("锚点缺失时保留说明并给出非阻塞提示，不抛错", () => {
    renderWizard(0);
    expect(screen.getByTestId("onboarding-step-title")).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-anchor-missing")).toBeInTheDocument();
    expect(screen.queryByTestId("onboarding-highlight")).not.toBeInTheDocument();
  });

  it("锚点存在时呈现高亮层", () => {
    const anchor = document.createElement("button");
    anchor.setAttribute("data-testid", "onboarding-anchor-project-create");
    document.body.appendChild(anchor);

    renderWizard(0);
    expect(screen.getByTestId("onboarding-highlight")).toBeInTheDocument();
    expect(screen.queryByTestId("onboarding-anchor-missing")).not.toBeInTheDocument();
  });
});

function mockElementRect(
  el: Element,
  rect: { top: number; left: number; width: number; height: number },
) {
  el.getBoundingClientRect = () => {
    const { top, left, width, height } = rect;
    return {
      top,
      left,
      width,
      height,
      right: left + width,
      bottom: top + height,
      x: left,
      y: top,
      toJSON: () => ({}),
    } as DOMRect;
  };
}

describe("OnboardingWizard 引导卡定位", () => {
  afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
  });

  it("卡片直接挂载在 body 下（portal 免疫 transform 祖先包含块）", () => {
    renderWizard(0);
    expect(screen.getByTestId("onboarding-wizard").parentElement).toBe(document.body);
  });

  it("有目标时卡片锚定到目标右侧并渲染指向标记", () => {
    const anchor = document.createElement("button");
    anchor.setAttribute("data-testid", "onboarding-anchor-issue-create");
    document.body.appendChild(anchor);
    // jsdom 视口 1024×768；卡片尺寸测得 0×0 → 首选右侧：left = 100+120+12。
    mockElementRect(anchor, { top: 200, left: 100, width: 120, height: 40 });

    renderWizard(1);
    const card = screen.getByTestId("onboarding-wizard");
    expect(card).toHaveAttribute("data-anchor-side", "right");
    expect(card.style.left).toBe("232px");
    expect(card.style.top).toBe("220px");
    expect(screen.getByTestId("onboarding-arrow")).toBeInTheDocument();
  });

  it("目标移动后 resize 触发重算", async () => {
    const anchor = document.createElement("button");
    anchor.setAttribute("data-testid", "onboarding-anchor-project-create");
    document.body.appendChild(anchor);
    mockElementRect(anchor, { top: 200, left: 100, width: 120, height: 40 });

    renderWizard(0);
    const card = screen.getByTestId("onboarding-wizard");
    expect(card.style.left).toBe("232px");

    mockElementRect(anchor, { top: 40, left: 800, width: 120, height: 40 });
    fireEvent.resize(window);
    await waitFor(() => expect(card.style.left).toBe("932px"));
    expect(card.style.top).toBe("60px");
  });

  it("无锚点时回退视口底部居中，不渲染指向标记", () => {
    renderWizard(0);
    const card = screen.getByTestId("onboarding-wizard");
    expect(card).toHaveAttribute("data-anchor-side", "fallback");
    expect(card.className).toContain("left-1/2");
    expect(screen.queryByTestId("onboarding-arrow")).not.toBeInTheDocument();
  });
});
