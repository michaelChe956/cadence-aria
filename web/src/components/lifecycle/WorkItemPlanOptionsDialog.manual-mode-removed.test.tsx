import { render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";

// REQ-WIGA-01 修订（work-item-group-manual-mode-removed）：已确认 design
// （automationAvailable === true，即「工作项组生成路径」）的 Work Item Plan
// 配置弹窗不提供「手动」生成模式——恒为托管自动化；目标投影不可得时提交
// 保持禁用，不提供任何以人工推进 workspace 会话创建工作项组的替代提交
// （含「创建并打开 Workspace」），可重试、可取消退出。

const SCOPE = { projectId: "project_0001", issueId: "issue_0001" } as const;

const BASE_OPTIONS: WorkItemPlanOptionsFormValue = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
  automation_mode: "manual",
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const LOGICAL_TARGET = {
  kind: "logical_codebase",
  logical_codebase_id: "11111111-1111-1111-1111-111111111111",
  logical_repository_id: "00000000-0000-0000-0000-000000000001",
} as const;

function targetResponse(): unknown {
  return {
    enrollment_target: LOGICAL_TARGET,
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
        require_execution_plan_confirm: false,
      },
    },
  };
}

/** 按 URL 分派：automation-target／automation-enrollment 之外一律抛错——
 * 断言「无人工 prepare 请求」时，任何额外请求都会显式失败而非被静默吞掉。 */
function stubFetch(targetStatus: "ok" | "fail"): Mock {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/automation-enrollment")) {
      return jsonResponse(null);
    }
    if (url.includes("/automation-target")) {
      return targetStatus === "ok"
        ? jsonResponse(targetResponse())
        : jsonResponse(
            {
              code: "automation_enrollment_invalid_scope",
              message: "automation target requires exactly one logical repository",
              details: {},
            },
            422,
          );
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function renderDialog(
  onConfirm: (options: WorkItemPlanOptionsFormValue) => Promise<void> = vi.fn(
    async () => undefined,
  ),
): HTMLElement {
  render(
    <WorkItemPlanOptionsDialog
      defaultOptions={BASE_OPTIONS}
      automationAvailable
      automationTargetScope={SCOPE}
      onConfirm={onConfirm}
      onClose={vi.fn()}
    />,
  );
  return screen.getByRole("dialog", { name: "Work Item Plan 配置" });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("WorkItemPlanOptionsDialog：工作项组生成路径不提供手动模式（REQ-WIGA-01 修订）", () => {
  it("work_item_group_dialog_offers_no_manual_mode", async () => {
    stubFetch("ok");
    const dialog = renderDialog();

    // 红测锚点：已确认 design 的弹窗不存在「手动」radio。
    expect(
      within(dialog).queryByRole("radio", { name: "手动" }),
    ).not.toBeInTheDocument();
    // 单选项「自动化」保留且恒为选中；提交形态恒为「启用自动化」。
    expect(
      within(dialog).getByRole("radio", { name: "自动化" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("button", { name: "启用自动化" }),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "创建并打开 Workspace" }),
    ).toBeNull();

    await within(dialog).findByTestId("automation-target-projection");
  });

  it("work_item_group_target_failure_blocks_submit_without_manual_bypass", async () => {
    const onConfirm = vi.fn(async () => undefined);
    const fetchMock = stubFetch("fail");
    const dialog = renderDialog(onConfirm);

    // GET 失败：如实报错＋可重试；提交保持禁用（fail-closed）。
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent(/自动化目标加载失败/);
    expect(within(dialog).getByTestId("automation-target-retry")).toBeEnabled();
    const submit = within(dialog).getByRole("button", {
      name: "启用自动化",
    });
    expect(submit).toBeDisabled();

    // D2（已终裁为 (b)）：无「创建并打开 Workspace」替代提交、无「手动」
    // 绕过、不发起人工 prepare 请求、onConfirm 未被调用；「取消」退出可用。
    expect(
      within(dialog).queryByRole("radio", { name: "手动" }),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "创建并打开 Workspace" }),
    ).toBeNull();
    expect(onConfirm).not.toHaveBeenCalled();
    expect(
      fetchMock.mock.calls.some(([url]) => String(url).includes("prepare")),
    ).toBe(false);
    expect(within(dialog).getByRole("button", { name: "取消" })).toBeEnabled();
  });
});
