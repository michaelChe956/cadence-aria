import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateProjectDialog } from "./CreateProjectDialog";

describe("CreateProjectDialog", () => {
  it("submits the project name and description without a repository mode", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn();
    render(<CreateProjectDialog onCreate={onCreate} onClose={vi.fn()} />);

    const dialog = screen.getByRole("dialog", { name: "新建 Project" });
    expect(within(dialog).queryByRole("radio")).toBeNull();
    await user.type(within(dialog).getByLabelText("Project 名称"), "Aria");
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Project" }),
    );

    expect(onCreate).toHaveBeenCalledWith({
      name: "Aria",
      description: null,
    });
  });
});
