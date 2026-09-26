import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppShell } from "./app-shell";
import { ONBOARDING_SEEN_KEY } from "./onboarding/useOnboarding";
import { CURRENT_VERSION } from "./whats-new/changelog";

vi.mock("./components/lifecycle/IssueLifecycleWorkbench", () => ({
  IssueLifecycleWorkbench: () => <div data-testid="workbench-stub" />,
}));

const SEEN_KEY = "aria-whats-new-seen";

function renderShell() {
  return render(
    <AppShell
      onDrawerFocusChange={() => {}}
      onOpenWorkspace={() => {}}
      onOpenCodingWorkspace={() => {}}
    />,
  );
}

describe("AppShell 版本更新弹窗", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("未读当前版本时弹出 WhatsNewDialog", () => {
    renderShell();
    expect(screen.getByRole("dialog", { name: "版本更新说明" })).toBeInTheDocument();
  });

  it("已读当前版本时不弹", () => {
    window.localStorage.setItem(SEEN_KEY, CURRENT_VERSION);
    renderShell();
    expect(screen.queryByRole("dialog", { name: "版本更新说明" })).not.toBeInTheDocument();
  });

  it("点击知道了后写入 localStorage 并关闭弹窗", async () => {
    const user = userEvent.setup();
    renderShell();
    await user.click(screen.getByRole("button", { name: "知道了" }));
    expect(window.localStorage.getItem(SEEN_KEY)).toBe(CURRENT_VERSION);
    expect(screen.queryByRole("dialog", { name: "版本更新说明" })).not.toBeInTheDocument();
  });
});

describe("AppShell 操作引导", () => {
  beforeEach(() => {
    window.localStorage.clear();
    // 抑制 WhatsNew，聚焦 onboarding 接线。
    window.localStorage.setItem(SEEN_KEY, CURRENT_VERSION);
  });

  it("首次进入（无已读）展示引导并从第一个启用步骤开始", () => {
    renderShell();
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-step-title")).toHaveTextContent("添加 Project");
    expect(screen.getByTestId("onboarding-progress")).toHaveTextContent("第 1 / 共 13 步");
  });

  it("同一设备再次进入不自动展示引导", () => {
    window.localStorage.setItem(ONBOARDING_SEEN_KEY, "1");
    renderShell();
    expect(screen.queryByTestId("onboarding-wizard")).not.toBeInTheDocument();
    expect(screen.getByTestId("onboarding-help-trigger")).toBeInTheDocument();
  });

  it("跳过后写入已读并关闭", async () => {
    const user = userEvent.setup();
    renderShell();
    await user.click(screen.getByTestId("onboarding-skip"));
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBe("1");
    expect(screen.queryByTestId("onboarding-wizard")).not.toBeInTheDocument();
  });

  it("帮助入口重开引导、回到第一步且不清除已读、不改业务面", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem(ONBOARDING_SEEN_KEY, "1");
    renderShell();
    expect(screen.queryByTestId("onboarding-wizard")).not.toBeInTheDocument();

    await user.click(screen.getByTestId("onboarding-help-trigger"));
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-progress")).toHaveTextContent("第 1 / 共 13 步");
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBe("1");
    // 工作台业务面照常存在，重开引导未改变它。
    expect(screen.getByTestId("workbench-stub")).toBeInTheDocument();
  });

  it("与版本更新弹窗并存：两者可同时呈现", () => {
    window.localStorage.clear();
    renderShell();
    expect(screen.getByRole("dialog", { name: "版本更新说明" })).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();
  });
});
