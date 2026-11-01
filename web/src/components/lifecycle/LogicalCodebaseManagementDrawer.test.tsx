// LC 管理面板抽屉化：右侧抽屉外壳单测（vitest + testing-library）。
// 契约：
// - 展开内容渲染在 role="dialog" 且 aria-modal="true" 的容器内（w-[min(560px,92vw)] 右侧滑入）；
// - Esc / 点遮罩 / 头部关闭按钮均触发 onClose；点面板本体不触发；
// - 打开时焦点移入抽屉，关闭卸载后归还触发元素；Tab 在面板内循环圈闭；
// - 打开期间 body 滚动锁定（overflow hidden），卸载后还原；
// - prefers-reduced-motion 降级：无过渡直切（类含 motion-reduce:transition-none）且行为不报错。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseManagementDrawer } from "./LogicalCodebaseManagementDrawer";

function stubReduceMotion(reduce: boolean) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: reduce && query.includes("prefers-reduced-motion"),
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
}

function Harness(props: { open?: boolean; onClose?: () => void }) {
  const [open, setOpen] = useState(props.open ?? false);
  const onClose = props.onClose ?? (() => setOpen(false));
  return (
    <>
      <button type="button" data-testid="opener" onClick={() => setOpen(true)}>
        打开管理
      </button>
      <LogicalCodebaseManagementDrawer open={open} onClose={onClose}>
        <div data-testid="lc-panel-body">
          <button type="button" data-testid="first-btn">
            第一步
          </button>
          <button type="button" data-testid="last-btn">
            最后一步
          </button>
        </div>
      </LogicalCodebaseManagementDrawer>
    </>
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("LogicalCodebaseManagementDrawer", () => {
  it("展开后内容渲染在 role=dialog 且 aria-modal=true 的右侧抽屉容器内", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(screen.getByTestId("opener"));

    const drawer = screen.getByRole("dialog", { name: "逻辑代码库管理" });
    expect(drawer).toHaveAttribute("aria-modal", "true");
    expect(drawer).toHaveClass("w-[min(560px,92vw)]");
    expect(screen.getByTestId("lc-panel-body")).toBeInstanceOf(HTMLElement);
    // 进入过渡后可视（translate-x-0），遮罩淡入。
    await waitFor(() => expect(drawer).toHaveClass("translate-x-0"));
    expect(screen.getByTestId("lc-management-overlay")).toHaveClass(
      "opacity-100",
    );
  });

  it("Esc 键触发 onClose", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    await waitFor(() =>
      expect(screen.getByRole("dialog")).toHaveClass("translate-x-0"),
    );

    await user.keyboard("{Escape}");

    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("点遮罩触发 onClose；点面板本体不触发", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<Harness onClose={onClose} />);

    await user.click(screen.getByTestId("opener"));
    await waitFor(() =>
      expect(screen.getByRole("dialog")).toHaveClass("translate-x-0"),
    );

    await user.click(screen.getByTestId("lc-panel-body"));
    expect(onClose).not.toHaveBeenCalled();

    await user.click(screen.getByTestId("lc-management-overlay"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("头部关闭按钮触发 onClose", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<Harness onClose={onClose} />);

    await user.click(screen.getByTestId("opener"));
    await user.click(await screen.findByTestId("lc-management-close"));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("打开时焦点移入抽屉，关闭卸载后归还触发元素", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const opener = screen.getByTestId("opener");
    await user.click(opener);
    const drawer = await screen.findByRole("dialog");
    await waitFor(() => expect(drawer).toHaveFocus());

    await user.click(screen.getByTestId("lc-management-close"));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(opener).toHaveFocus();
  });

  it("Tab 在抽屉内循环圈闭：末位 Tab 回首位，首位 Shift+Tab 回末位", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    await screen.findByRole("dialog");

    // 打开后焦点在面板容器上，Tab 落到面板内首个可聚焦元素（头部关闭按钮）。
    await user.tab();
    expect(screen.getByTestId("lc-management-close")).toHaveFocus();

    // 末位（last-btn）Tab → 圈回首位（关闭按钮）。
    await user.click(screen.getByTestId("last-btn"));
    await user.tab();
    expect(screen.getByTestId("lc-management-close")).toHaveFocus();

    // 首位（关闭按钮）Shift+Tab → 圈回末位（last-btn）。
    await user.tab({ shift: true });
    expect(screen.getByTestId("last-btn")).toHaveFocus();
  });

  it("打开期间 body 滚动锁定，卸载后还原", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    await screen.findByRole("dialog");
    expect(document.body.style.overflow).toBe("hidden");

    await user.click(screen.getByTestId("lc-management-close"));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(document.body.style.overflow).toBe("");
  });

  it("motion-reduce 降级：无过渡类直切，行为不报错", async () => {
    stubReduceMotion(true);
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    const drawer = await screen.findByRole("dialog");
    expect(drawer).toHaveClass("motion-reduce:transition-none");

    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(screen.getByTestId("opener")).toHaveFocus();
  });

  it("关闭走退场过渡：shown 类回退后内容才卸载", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    await waitFor(() =>
      expect(screen.getByRole("dialog")).toHaveClass("translate-x-0"),
    );

    await user.click(screen.getByTestId("lc-management-close"));
    // 退场期间仍挂载（translate-x-full），过渡结束后卸载。
    expect(screen.getByRole("dialog")).toHaveClass("translate-x-full");
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });
});
