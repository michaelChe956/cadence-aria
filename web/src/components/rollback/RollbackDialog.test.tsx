import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RollbackDialog } from "./RollbackDialog";

describe("RollbackDialog", () => {
  it("requires dirty confirmation before rollback", async () => {
    const onConfirm = vi.fn();
    render(
      <RollbackDialog
        open
        preview={{
          checkpoint_id: "ckpt_0001",
          git_head: "abc1234",
          dirty: true,
          turns_to_drop: 3,
          node_runs_to_drop: 7,
          provider_runs_to_drop: 4,
          artifacts_to_drop: 6,
          files_may_change: ["src/fibonacciSquareSum.js"],
        }}
        onConfirm={onConfirm}
        onOpenChange={() => undefined}
      />,
    );

    expect(screen.getByRole("button", { name: "执行回退" })).toBeDisabled();
    await userEvent.click(screen.getByLabelText("允许丢弃当前未提交变更"));
    await userEvent.click(screen.getByRole("button", { name: "执行回退" }));
    expect(onConfirm).toHaveBeenCalledWith({
      checkpoint_id: "ckpt_0001",
      force_when_dirty: true,
    });
  });
});
