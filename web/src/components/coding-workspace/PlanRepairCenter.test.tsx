import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PlanRepairCenter } from "./PlanRepairCenter";
import { repairAwaitingConfirmationFixture } from "./plan-repair-test-fixtures";
import { linkedWorkspaceAmendmentSnapshotFixture } from "./plan-repair-test-fixtures";

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

  it("opens Story and Design target selection inside adjust scope without adding a root action", async () => {
    const onStartLinkedAmendment = vi.fn(() => true);
    render(
      <PlanRepairCenter
        state={repairAwaitingConfirmationFixture()}
        onAction={vi.fn()}
        linkedAmendmentTargets={{
          story: ["story_spec_0001"],
          design: ["design_spec_0001"],
        }}
        onStartLinkedAmendment={onStartLinkedAmendment}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "调整修订范围" }));
    expect(screen.getByRole("group", { name: "Plan Repair 操作" }).querySelectorAll("button, a"))
      .toHaveLength(5);
    expect(screen.getByRole("group", { name: "Story/Design 修订目标" }))
      .toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("修订类型"), "design");
    await userEvent.click(screen.getByRole("button", { name: "发起关联修订" }));

    expect(onStartLinkedAmendment).toHaveBeenCalledWith({
      entity_id: "design_spec_0001",
      workspace_type: "design",
      relation: "design_amendment",
    });
  });

  it("renders a safe child link only after a matching ready snapshot", async () => {
    const state = repairAwaitingConfirmationFixture();
    render(
      <PlanRepairCenter
        state={state}
        onAction={vi.fn()}
        linkedAmendmentTargets={{ story: ["story_spec_0001"], design: [] }}
        linkedAmendmentStatus="ready"
        linkedAmendmentSnapshot={linkedWorkspaceAmendmentSnapshotFixture()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "调整修订范围" }));
    expect(screen.getByRole("link", { name: "打开已创建的 Story Workspace" }))
      .toHaveAttribute(
        "href",
        "/workbench/workspace/workspace_session_story_amendment_0001",
      );
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

  it("disables every mutation while an authoritative repair action is pending", () => {
    render(
      <PlanRepairCenter
        state={repairAwaitingConfirmationFixture()}
        onAction={vi.fn()}
        actionPending
        actionStatus="正在提交 Repair 操作，等待 Child Workspace 响应。"
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
  });

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
