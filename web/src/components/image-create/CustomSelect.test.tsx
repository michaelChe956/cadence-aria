import { render, screen } from "@testing-library/react";
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
});
