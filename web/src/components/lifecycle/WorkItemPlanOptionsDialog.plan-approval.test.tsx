import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";

// B2-P1 前端段 F2 修复（REQ-WIGA-02；k3 波1 复审）：automatic 模式下
// 「子 Work Item 执行前需要确认 Plan」复选框置灰＋固定关闭显示与说明
// ——后端 auto 冻结一律压 false（`normalized_for_enrollment_freeze`），
// 可勾选但 100% 无效且无提示属缺陷；manual 模式行为不变。

const CONFIRM_CHECKBOX_NAME = "子 Work Item 执行前需要确认 Plan";

const SCOPE = { projectId: "project_0001", issueId: "issue_0001" } as const;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function stubTargetFetch(): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      jsonResponse({
        enrollment_target: {
          kind: "single_repository",
          repository_id: "00000000-0000-0000-0000-000000000001",
        },
        resolved_options: {
          author_provider: "codex",
          reviewer_provider: "claude_code",
          review_rounds: 1,
          superpowers_enabled: true,
          openspec_enabled: true,
          plan_approval: "manual",
          plan_options: {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: true,
            require_execution_plan_confirm: true,
          },
        },
      }),
    ),
  );
}

function baseOptions(
  requireExecutionPlanConfirm: boolean,
): WorkItemPlanOptionsFormValue {
  return {
    include_integration_tests: true,
    include_e2e_tests: false,
    force_frontend_backend_split: true,
    require_execution_plan_confirm: requireExecutionPlanConfirm,
    automation_mode: "manual",
  };
}

function renderDialog(defaultOptions: WorkItemPlanOptionsFormValue): HTMLElement {
  render(
    <WorkItemPlanOptionsDialog
      defaultOptions={defaultOptions}
      automationAvailable
      automationTargetScope={SCOPE}
      onConfirm={vi.fn(async () => undefined)}
      onClose={vi.fn()}
    />,
  );
  return screen.getByRole("dialog", { name: "Work Item Plan 配置" });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("WorkItemPlanOptionsDialog automatic 模式的执行前确认复选框（B2-P1 F2）", () => {
  it("disables the checkbox with a fixed-off note in automatic mode and restores it in manual mode", async () => {
    const user = userEvent.setup();
    stubTargetFetch();
    // 表单值保持 true：证明 automatic 下是「固定关闭显示」，不篡改表单值，
    // 切回手动即恢复用户原勾选。
    const dialog = renderDialog(baseOptions(true));
    const checkbox = () =>
      within(dialog).getByRole("checkbox", {
        name: CONFIRM_CHECKBOX_NAME,
      });

    expect(checkbox()).toBeEnabled();
    expect(checkbox()).toBeChecked();
    expect(
      within(dialog).queryByTestId("plan-approval-auto-note"),
    ).toBeNull();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-target-projection");

    // 红测锚点：automatic 下置灰＋固定关闭＋说明可见。
    expect(checkbox()).toBeDisabled();
    expect(checkbox()).not.toBeChecked();
    const note = within(dialog).getByTestId("plan-approval-auto-note");
    expect(note).toHaveTextContent(/自动化模式/);
    expect(note).toHaveTextContent(/自动批准/);

    // 切回手动：复选框恢复可用与原勾选，说明消失（manual 不变）。
    await user.click(within(dialog).getByRole("radio", { name: "手动" }));
    expect(checkbox()).toBeEnabled();
    expect(checkbox()).toBeChecked();
    expect(
      within(dialog).queryByTestId("plan-approval-auto-note"),
    ).toBeNull();
  });

  it("keeps the checkbox editable per its form value in manual mode without any note", async () => {
    const user = userEvent.setup();
    stubTargetFetch();
    const dialog = renderDialog(baseOptions(true));

    const checkbox = within(dialog).getByRole("checkbox", {
      name: CONFIRM_CHECKBOX_NAME,
    });
    expect(checkbox).toBeEnabled();
    expect(checkbox).toBeChecked();
    expect(
      within(dialog).queryByTestId("plan-approval-auto-note"),
    ).toBeNull();

    await user.click(checkbox);
    expect(
      within(dialog).getByRole("checkbox", { name: CONFIRM_CHECKBOX_NAME }),
    ).not.toBeChecked();
  });
});
