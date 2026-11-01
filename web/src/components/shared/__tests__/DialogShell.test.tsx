// DialogShell 居中弹窗外壳单测（vitest + testing-library）。
// 契约（对齐 LogicalCodebaseManagementDrawer 的既有浮层规范）：
// - 内容渲染在 role="dialog" 且 aria-modal="true" 的卡片内，标题栏 text-sm font-semibold + h-7 w-7 X 按钮；
// - 层 portal 到 document.body（免疫 transform/filter 祖先导致的 fixed 失效——滚动后蒙层错位根因）；
// - 层 fixed inset-0，蒙层 absolute inset-0 bg-black/35（父层 fixed → 恒铺满视口）；
// - 内容滚动区 overscroll-contain（滚轮不穿透/不链到页面），卡片 max-h-[85vh] 内部滚动；
// - 打开时 body overflow hidden，卸载还原；Esc / 点蒙层 / X 关闭，closeDisabled 时三路都禁用；
// - 打开焦点移入卡片，卸载归还触发元素；Tab 在卡片内循环圈闭；motion-reduce 无过渡直切。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DialogShell } from "../DialogShell";

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

function Harness(props: {
  onClose?: (() => void) | null;
  closeDisabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const onClose =
    props.onClose === undefined ? () => setOpen(false) : props.onClose;
  return (
    <>
      <button type="button" data-testid="opener" onClick={() => setOpen(true)}>
        打开弹窗
      </button>
      {open ? (
        <DialogShell
          label="示例弹窗"
          onClose={onClose}
          closeDisabled={props.closeDisabled}
        >
          <div data-testid="shell-body">
            <button type="button" data-testid="first-btn">
              第一步
            </button>
            <button type="button" data-testid="last-btn">
              最后一步
            </button>
          </div>
        </DialogShell>
      ) : null}
    </>
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("DialogShell", () => {
  it("渲染 role=dialog 卡片：aria-modal、标题栏与 h-7 w-7 X 关闭按钮", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));

    const dialog = screen.getByRole("dialog", { name: "示例弹窗" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(
      screen.getByRole("heading", { name: "示例弹窗" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "关闭示例弹窗" }),
    ).toHaveClass("h-7", "w-7");
    expect(screen.getByTestId("shell-body")).toBeInTheDocument();
  });

  it("层 portal 到 document.body：transform 祖先不再劫持 fixed 蒙层", async () => {
    const user = userEvent.setup();
    render(
      // 祖先带常驻 translate（Tailwind translate 属性非 none）时，非 portal 的
      // fixed 子层会以该祖先为包含块——页面滚动后蒙层跟着滚走（用户症状二）。
      <div className="translate-x-0" data-testid="transformed-host">
        <Harness />
      </div>,
    );

    await user.click(screen.getByTestId("opener"));

    const dialog = screen.getByRole("dialog", { name: "示例弹窗" });
    const layer = dialog.closest(".fixed.inset-0") as HTMLElement;
    expect(layer).not.toBeNull();
    // 层是 document.body 的直接子节点（portal 逃出了 transform 祖先）。
    expect(layer.parentElement).toBe(document.body);
    // 蒙层位于 fixed 层内的 absolute inset-0（父层 fixed → 恒铺满视口）。
    const overlay = layer.querySelector(
      ":scope > div[aria-hidden='true']",
    ) as HTMLElement;
    expect(overlay).not.toBeNull();
    expect(overlay).toHaveClass("absolute", "inset-0", "bg-black/35");
  });

  it("卡片长内容内部滚动：max-h + overscroll-contain 滚动区", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));

    const dialog = screen.getByRole("dialog", { name: "示例弹窗" });
    expect(dialog).toHaveClass("max-h-[85vh]", "overflow-hidden");
    expect(screen.getByTestId("shell-body").parentElement).toHaveClass(
      "min-h-0",
      "flex-1",
      "overflow-y-auto",
      "overscroll-contain",
    );
  });

  it("打开期间 body 滚动锁定，卸载后还原", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    expect(document.body.style.overflow).toBe("hidden");

    await user.click(screen.getByRole("button", { name: "关闭示例弹窗" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(document.body.style.overflow).toBe("");
  });

  it("Esc / 点蒙层 / X 三路关闭；点卡片本体不关闭", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<Harness onClose={onClose} />);

    await user.click(screen.getByTestId("opener"));
    const dialog = screen.getByRole("dialog", { name: "示例弹窗" });

    await user.click(screen.getByTestId("shell-body"));
    expect(onClose).not.toHaveBeenCalled();

    const overlay = dialog
      .closest(".fixed.inset-0")!
      .querySelector(":scope > div[aria-hidden='true']")!;
    await user.click(overlay);
    expect(onClose).toHaveBeenCalledTimes(1);

    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(2);
    // spy 不卸载宿主，卡片仍在；真实宿主 setOpen(false) 后由上一用例覆盖卸载路径。
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("closeDisabled 时 X 禁用且 Esc/蒙层不关闭", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<Harness onClose={onClose} closeDisabled />);

    await user.click(screen.getByTestId("opener"));
    expect(
      screen.getByRole("button", { name: "关闭示例弹窗" }),
    ).toBeDisabled();

    await user.keyboard("{Escape}");
    const dialog = screen.getByRole("dialog", { name: "示例弹窗" });
    const overlay = dialog
      .closest(".fixed.inset-0")!
      .querySelector(":scope > div[aria-hidden='true']")!;
    await user.click(overlay);
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("打开时焦点移入卡片，关闭卸载后归还触发元素", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const opener = screen.getByTestId("opener");
    await user.click(opener);
    const dialog = await screen.findByRole("dialog");
    await waitFor(() => expect(dialog).toHaveFocus());

    await user.click(screen.getByRole("button", { name: "关闭示例弹窗" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(opener).toHaveFocus();
  });

  it("Tab 在卡片内循环圈闭：末位 Tab 回首位，首位 Shift+Tab 回末位", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    await screen.findByRole("dialog");

    // 打开后焦点在卡片容器上，Tab 落到卡片内首个可聚焦元素（头部关闭按钮）。
    await user.tab();
    expect(
      screen.getByRole("button", { name: "关闭示例弹窗" }),
    ).toHaveFocus();

    // 末位（last-btn）Tab → 圈回首位（关闭按钮）。
    await user.click(screen.getByTestId("last-btn"));
    await user.tab();
    expect(
      screen.getByRole("button", { name: "关闭示例弹窗" }),
    ).toHaveFocus();

    // 首位（关闭按钮）Shift+Tab → 圈回末位（last-btn）。
    await user.tab({ shift: true });
    expect(screen.getByTestId("last-btn")).toHaveFocus();
  });

  it("进场过渡：200ms opacity/transform 类挂载，跨帧后进入可视态", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    const dialog = screen.getByRole("dialog", { name: "示例弹窗" });
    expect(dialog).toHaveClass(
      "transition-[opacity,transform]",
      "duration-200",
    );
    const overlay = dialog
      .closest(".fixed.inset-0")!
      .querySelector(":scope > div[aria-hidden='true']")!;
    expect(overlay).toHaveClass(
      "transition-opacity",
      "duration-200",
      "bg-black/35",
    );

    await waitFor(() => expect(dialog).toHaveClass("opacity-100", "scale-100"));
    expect(overlay).toHaveClass("opacity-100");
  });

  it("motion-reduce 降级：无过渡直切类保留，行为不报错", async () => {
    stubReduceMotion(true);
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByTestId("opener"));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveClass("motion-reduce:transition-none");

    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(screen.getByTestId("opener")).toHaveFocus();
  });
});
