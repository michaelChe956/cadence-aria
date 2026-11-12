import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { CustomSelect } from "./CustomSelect";

const OPTIONS = ["auto", "high", "low"] as const;

function ControlledSelect({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState("auto");
  return (
    <CustomSelect
      label="质量"
      value={value}
      options={OPTIONS}
      disabled={disabled}
      onChange={setValue}
    />
  );
}

describe("CustomSelect", () => {
  it("opens a styled listbox and selects an option", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <CustomSelect
        label="质量"
        value="auto"
        options={OPTIONS}
        onChange={onChange}
      />,
    );

    const combobox = screen.getByRole("combobox", { name: "质量" });
    expect(combobox).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();

    await user.click(combobox);

    expect(combobox).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("listbox", { name: "质量" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "auto" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    await user.click(screen.getByRole("option", { name: "high" }));

    expect(onChange).toHaveBeenCalledWith("high");
    expect(combobox).toHaveAttribute("aria-expanded", "false");
    expect(combobox).toHaveFocus();
  });

  it("closes on Escape and outside mousedown", async () => {
    const user = userEvent.setup();
    render(
      <div>
        <ControlledSelect />
        <button type="button">外部按钮</button>
      </div>,
    );

    const combobox = screen.getByRole("combobox", { name: "质量" });
    await user.click(combobox);
    await user.keyboard("{Escape}");
    expect(combobox).toHaveAttribute("aria-expanded", "false");

    await user.click(combobox);
    await user.click(screen.getByRole("button", { name: "外部按钮" }));
    expect(combobox).toHaveAttribute("aria-expanded", "false");
  });

  it("does not open while disabled", async () => {
    const user = userEvent.setup();
    render(<ControlledSelect disabled />);

    const combobox = screen.getByRole("combobox", { name: "质量" });
    expect(combobox).toBeDisabled();
    await user.click(combobox);
    expect(combobox).toHaveAttribute("aria-expanded", "false");
  });

  it("shows displayValue on the trigger instead of the raw value", () => {
    render(
      <CustomSelect
        label="模型"
        value="gpt-image-2.6"
        displayValue="推荐模型"
        options={OPTIONS}
        onChange={() => {}}
      />,
    );

    const combobox = screen.getByRole("combobox", { name: "模型" });
    expect(combobox).toHaveTextContent("推荐模型");
    expect(combobox).not.toHaveTextContent(/gpt-image/);
  });
  it("renders the option reason on its own line instead of dash concatenation", async () => {
    const user = userEvent.setup();
    render(
      <CustomSelect
        label="Provider"
        value="codex"
        options={[
          { value: "codex", label: "Codex", reason: "Provider 状态尚未确认" },
        ]}
        onChange={() => {}}
      />,
    );

    await user.click(screen.getByRole("combobox", { name: "Provider" }));

    const option = screen.getByRole("option", { name: /^Codex/ });
    const label = within(option).getByText("Codex");
    const reason = within(option).getByText("Provider 状态尚未确认");
    expect(label).not.toBe(reason);
    expect(label.textContent).toBe("Codex");
    expect(reason.textContent).toBe("Provider 状态尚未确认");
    expect(option.textContent).not.toContain("—");
  });

  it("portals the listbox to document.body so overflow ancestors cannot clip it", async () => {
    const user = userEvent.setup();
    render(
      <div className="overflow-y-auto overscroll-contain">
        <CustomSelect
          label="质量"
          value="auto"
          options={OPTIONS}
          onChange={() => {}}
        />
      </div>,
    );

    await user.click(screen.getByRole("combobox", { name: "质量" }));

    const listbox = screen.getByRole("listbox", { name: "质量" });
    expect(listbox.parentElement?.closest(".overflow-y-auto")).toBeNull();
    expect(listbox.parentElement).toBe(document.body);
    expect(listbox.className).toContain("fixed");
  });

  it("treats the portaled list as inside the outside-close boundary", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <CustomSelect
        label="质量"
        value="auto"
        options={OPTIONS}
        onChange={onChange}
      />,
    );

    const combobox = screen.getByRole("combobox", { name: "质量" });
    await user.click(combobox);

    fireEvent.mouseDown(screen.getByRole("listbox", { name: "质量" }));
    expect(combobox).toHaveAttribute("aria-expanded", "true");

    await user.click(screen.getByRole("option", { name: "high" }));
    expect(onChange).toHaveBeenCalledWith("high");
  });

  it("打开后下拉面板进入可见态（拿到 opacity-100，而不是停在 opacity-0）", async () => {
    // 回归：面板 portal 后要算定位坐标 ⇒ 打开时会多一次渲染；若可见性用的
    // rAF 被「无依赖数组的卸载清理」在渲染间取消，面板会永远停在 opacity-0
    // （用户看到的是「点了没反应」）。
    const user = userEvent.setup();
    render(
      <CustomSelect
        label="质量"
        value="auto"
        options={OPTIONS}
        onChange={() => {}}
      />,
    );

    await user.click(screen.getByRole("combobox", { name: "质量" }));

    const listbox = screen.getByRole("listbox", { name: "质量" });
    await waitFor(() => {
      expect(listbox.className).toContain("opacity-100");
      expect(listbox.className).not.toContain("opacity-0");
      expect(listbox.className).not.toContain("pointer-events-none");
    });
  });

  it("关闭后下拉面板在过场结束后从 DOM 卸载", async () => {
    // 回归：关闭靠一个 150ms 定时器把 renderList 置 false；该定时器若被
    // 「无依赖数组的卸载清理」在渲染间取消，面板会永久留在 DOM 里
    // （看不见、也点不到）。
    const user = userEvent.setup();
    render(
      <CustomSelect
        label="质量"
        value="auto"
        options={OPTIONS}
        onChange={() => {}}
      />,
    );

    const combobox = screen.getByRole("combobox", { name: "质量" });
    await user.click(combobox);
    expect(screen.getByRole("listbox", { name: "质量" })).toBeInTheDocument();

    await user.click(combobox);

    await waitFor(() => {
      expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    });
  });
});
