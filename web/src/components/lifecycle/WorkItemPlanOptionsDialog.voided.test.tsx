import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { providerHealthSnapshot } from "../../state/provider-availability-test-fixtures";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";

// REQ-WIGA-VOID-01 前端段（tasks 5.2）：登记已作废（工作项组删除所致）与
// 已终止是两个独立维度——弹窗作废态显示「已随旧工作项组删除作废；确认后
// 将重新开启」，终止态保持既有「已于 X 终止」文案逐字不变；两种状态的
// 文案可区分，作废态不冒充终止态。

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

/** 已终止（未作废）的 enrollment 投影——与既有 terminated 夹具同构。 */
function terminatedEnrollment(): unknown {
  return {
    enrollment_id: "enrollment_0001",
    selection_key: "selection_old",
    project_id: "project_0001",
    issue_id: "issue_0001",
    enabled: false,
    policy_revision: 3,
    source: { stories: [], designs: [] },
    options: {
      author_provider: "kimi_code",
      reviewer_provider: "pi",
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      plan_approval: "auto",
      plan_options: {
        include_integration_tests: true,
        include_e2e_tests: false,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
      },
    },
    target: LOGICAL_TARGET,
    prepare_intent_id: "prepare_intent_0001",
    plan_id: null,
    session_id: null,
    created_at: "2026-10-16T00:00:00Z",
    updated_at: "2026-11-06T00:00:00Z",
    terminated_at: "2026-11-06T00:00:00Z",
  };
}

/** 已作废的 enrollment 投影（REQ-WIGA-VOID-01）：随工作项组删除作废——
 * 作废与终止独立维度并存（删除路径先终止再作废的正常终态）。 */
function voidedEnrollment(): unknown {
  return {
    ...(terminatedEnrollment() as Record<string, unknown>),
    voided_at: "2026-11-06T01:00:00Z",
    void_reason: "work_item_group_deleted",
  };
}

function stubFetch(enrollment: unknown): Mock {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/automation-target")) {
      return jsonResponse(targetResponse());
    }
    if (url.includes("/automation-enrollment")) {
      return jsonResponse(enrollment);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function renderDialog(): HTMLElement {
  render(
    <WorkItemPlanOptionsDialog
      defaultOptions={BASE_OPTIONS}
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
  useProviderAvailabilityStore.getState().reset();
});

describe("WorkItemPlanOptionsDialog：作废态（已随工作项组删除作废）", () => {
  it("states the voided fact instead of the terminated notice", async () => {
    const user = userEvent.setup();
    stubFetch(voidedEnrollment());
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    const notice = await within(dialog).findByTestId("automation-voided-notice");

    // 红测锚点：作废态如实显示「已随旧工作项组删除作废；确认后将重新开启」，
    // MUST NOT 一律显示为「已于 X 终止」（REQ-WIGA-VOID-01 场景五）。
    expect(notice).toHaveTextContent(
      "该 issue 的自动化登记已随旧工作项组删除作废；确认后将重新开启",
    );
    expect(
      within(dialog).queryByTestId("automation-terminated-notice"),
    ).toBeNull();
  });

  it("keeps the re-enable entry usable with providers selectable", async () => {
    const user = userEvent.setup();
    stubFetch(voidedEnrollment());
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-voided-notice");

    // 重新开启入口不阻断：参数置灰同 terminated 语义，provider 选择器可用
    //（作废态换 provider 一次完成——后端已放行全部漂移）。
    expect(
      within(dialog).getByRole("checkbox", { name: "包含贯通/集成测试 Work Item" }),
    ).toBeDisabled();
    expect(within(dialog).getByLabelText("Author Provider")).toBeEnabled();
    expect(within(dialog).getByLabelText("Reviewer Provider")).toBeEnabled();
    const submit = within(dialog).getByRole("button", { name: "启用自动化" });
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("keeps the terminated notice verbatim for a terminated-only enrollment", async () => {
    const user = userEvent.setup();
    stubFetch(terminatedEnrollment());
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    const notice = await within(dialog).findByTestId("automation-terminated-notice");

    // 既有终止文案逐字不变；两种状态可区分（作废态不渲染终止文案）。
    expect(notice).toHaveTextContent(
      /该 issue 的自动化已于 2026-11-06T00:00:00Z 终止；\s*确认「启用自动化」将重新开启（生成新的自动化授权，原自动化不恢复）。/,
    );
    expect(
      within(dialog).queryByTestId("automation-voided-notice"),
    ).toBeNull();
  });
});
