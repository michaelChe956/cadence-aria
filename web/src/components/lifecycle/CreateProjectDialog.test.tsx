import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateProjectDialog } from "./CreateProjectDialog";

describe("CreateProjectDialog", () => {
  it("默认选中单仓库（默认）并提交 multi_repo=false", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn();
    render(<CreateProjectDialog onCreate={onCreate} onClose={vi.fn()} />);

    const dialog = screen.getByRole("dialog", { name: "新建 Project" });
    expect(
      within(dialog).getByRole("radio", { name: "单仓库（默认）" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("radio", { name: "多仓库" }),
    ).not.toBeChecked();

    await user.type(within(dialog).getByLabelText("Project 名称"), "Aria");
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Project" }),
    );

    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith({
      name: "Aria",
      description: null,
      multi_repo: false,
    });
  });

  it("选择多仓库后提交 multi_repo=true", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn();
    render(<CreateProjectDialog onCreate={onCreate} onClose={vi.fn()} />);

    const dialog = screen.getByRole("dialog", { name: "新建 Project" });
    await user.type(within(dialog).getByLabelText("Project 名称"), "Aria");
    await user.click(within(dialog).getByRole("radio", { name: "多仓库" }));

    expect(
      within(dialog).getByRole("radio", { name: "多仓库" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("radio", { name: "单仓库（默认）" }),
    ).not.toBeChecked();

    await user.click(
      within(dialog).getByRole("button", { name: "创建 Project" }),
    );

    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith({
      name: "Aria",
      description: null,
      multi_repo: true,
    });
  });
});
