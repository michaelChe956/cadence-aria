import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { GateFeedbackEditor } from "./GateFeedbackEditor";

describe("GateFeedbackEditor", () => {
  it("shares trimmed feedback submit behavior for input and textarea variants", async () => {
    const onSubmit = vi.fn();
    render(
      <GateFeedbackEditor
        multiline
        value="  修复边界  "
        onChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "提交反馈" }));
    expect(onSubmit).toHaveBeenCalledWith("修复边界");
  });
});
