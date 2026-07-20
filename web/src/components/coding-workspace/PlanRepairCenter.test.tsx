import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PlanRepairCenter } from "./PlanRepairCenter";
import { repairAwaitingConfirmationFixture } from "./plan-repair-test-fixtures";

describe("PlanRepairCenter", () => {
  it("shows one amendment confirmation with impact and projection diffs", () => {
    render(<PlanRepairCenter state={repairAwaitingConfirmationFixture()} />);

    expect(
      screen.getByRole("button", { name: "确认修订并恢复执行" }),
    ).toBeInTheDocument();
    expect(screen.getByText("WI-01 初始化领域模型")).toBeInTheDocument();
    expect(screen.getByText("WI-01：重新执行")).toBeInTheDocument();
    expect(screen.getByText("WI-02：重新验证")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "确认 Coder Projection" }))
      .not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "确认 Reviewer Projection" }))
      .not.toBeInTheDocument();
  });

  it("exposes exactly the five approved user actions", async () => {
    const onAction = vi.fn();
    render(
      <PlanRepairCenter
        state={repairAwaitingConfirmationFixture()}
        onAction={onAction}
      />,
    );

    const actions = screen.getByRole("group", { name: "Plan Repair 操作" });
    expect(actions.querySelectorAll("button, a")).toHaveLength(5);

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(onAction).toHaveBeenCalledWith("confirm");
  });

  it.each([
    "authoring_revision",
    "validating_contract",
    "plan_review",
  ] as const)(
    "disables mutation actions during %s while keeping the full workspace link available",
    (stage) => {
      render(
        <PlanRepairCenter
          state={{ ...repairAwaitingConfirmationFixture(), stage }}
          onAction={vi.fn()}
        />,
      );

      for (const name of [
        "确认修订并恢复执行",
        "要求重新生成",
        "调整修订范围",
        "取消修订",
      ]) {
        expect(screen.getByRole("button", { name })).toBeDisabled();
      }
      expect(
        screen.getByRole("link", { name: "在完整 Work Item Workspace 中打开" }),
      ).toHaveAttribute("target", "_blank");
    },
  );

  it("uses semantic tabs with keyboard navigation and shared projection diff rendering", () => {
    render(<PlanRepairCenter state={repairAwaitingConfirmationFixture()} />);

    const summaryTab = screen.getByRole("tab", { name: "修订摘要" });
    summaryTab.focus();
    fireEvent.keyDown(summaryTab, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "Contract Diff" })).toHaveFocus();
    expect(screen.getByText("新增 failure_message")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Coder Diff" }));
    expect(
      screen.getByRole("heading", { name: "Coder Projection 影响" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Reviewer Diff" }));
    expect(
      screen.getByRole("heading", { name: "Reviewer Projection 影响" }),
    ).toBeInTheDocument();
  });
});
