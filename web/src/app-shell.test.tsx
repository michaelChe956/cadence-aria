import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppShell } from "./app-shell";
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
