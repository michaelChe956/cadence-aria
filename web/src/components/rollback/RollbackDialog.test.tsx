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

    expect(screen.getByRole("dialog")).toHaveTextContent("ckpt_0001");
    expect(screen.getByRole("button", { name: "执行回退" })).toHaveClass("bg-red-600");
    expect(screen.getByRole("button", { name: "执行回退" })).toBeDisabled();
    await userEvent.click(screen.getByLabelText("允许丢弃当前未提交变更"));
    await userEvent.click(screen.getByRole("button", { name: "执行回退" }));
    expect(onConfirm).toHaveBeenCalledWith({
      checkpoint_id: "ckpt_0001",
      force_when_dirty: true,
    });
  });

  it("resets dirty confirmation when checkpoint changes", async () => {
    const { rerender } = render(
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
        onConfirm={vi.fn()}
        onOpenChange={() => undefined}
      />,
    );

    await userEvent.click(screen.getByLabelText("允许丢弃当前未提交变更"));
    expect(screen.getByRole("button", { name: "执行回退" })).toBeEnabled();

    rerender(
      <RollbackDialog
        open
        preview={{
          checkpoint_id: "ckpt_0002",
          git_head: "def5678",
          dirty: true,
          turns_to_drop: 1,
          node_runs_to_drop: 2,
          provider_runs_to_drop: 1,
          artifacts_to_drop: 0,
          files_may_change: ["src/another.js"],
        }}
        onConfirm={vi.fn()}
        onOpenChange={() => undefined}
      />,
    );

    expect(screen.getByRole("dialog")).toHaveTextContent("ckpt_0002");
    expect(screen.getByLabelText("允许丢弃当前未提交变更")).not.toBeChecked();
    expect(screen.getByRole("button", { name: "执行回退" })).toBeDisabled();
  });
});
