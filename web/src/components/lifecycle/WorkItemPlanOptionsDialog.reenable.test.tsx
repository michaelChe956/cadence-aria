import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";

// 批 2 T4（REQ-WIGA-09 修订）前端段：terminated 态的「启动自动化」入口
// 仍可用——弹窗补读当前 enrollment 只读投影，如实告知用户该自动化已被
// 终止、确认即显式重新开启；入口不被终止态阻断（提交链路自带权威 GET
// 与服务端判定），投影读取失败也只是少一句说明，不猜状态、不阻断提交。

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

function targetResponse(): unknown {
  return {
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
        require_execution_plan_confirm: false,
      },
    },
  };
}

function terminatedEnrollment(): unknown {
  return {
    enrollment_id: "enrollment_0001",
    selection_key: "selection_old",
    project_id: "project_0001",
    issue_id: "issue_0001",
    enabled: false,
    policy_revision: 3,
    source: { stories: [], designs: [] },
    options: {},
    target: {
      kind: "single_repository",
      repository_id: "00000000-0000-0000-0000-000000000001",
    },
    prepare_intent_id: "prepare_intent_0001",
    plan_id: null,
    session_id: null,
    created_at: "2026-10-16T00:00:00Z",
    updated_at: "2026-10-20T00:00:00Z",
    terminated_at: "2026-10-20T00:00:00Z",
  };
}

/** enrollment 投影按用例给定：null（无记录）、terminated 记录或读取失败。 */
function stubFetch(enrollment: "none" | "terminated" | "error"): Mock {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/automation-target")) {
      return jsonResponse(targetResponse());
    }
    if (url.includes("/automation-enrollment")) {
      if (enrollment === "terminated") {
        return jsonResponse(terminatedEnrollment());
      }
      if (enrollment === "error") {
        return jsonResponse({ code: "boom", message: "boom" }, 500);
      }
      return jsonResponse(null);
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
});

describe("WorkItemPlanOptionsDialog：terminated 态的自动化入口", () => {
  it("states the termination and keeps the automatic submit available", async () => {
    const user = userEvent.setup();
    stubFetch("terminated");
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-target-projection");

    // 红测锚点：terminated 事实被如实告知，且入口不被阻断。
    const notice = await within(dialog).findByTestId(
      "automation-terminated-notice",
    );
    expect(notice).toHaveTextContent("2026-10-20T00:00:00Z");
    expect(notice).toHaveTextContent(/终止/);
    expect(notice).toHaveTextContent(/重新开启/);
    const submit = within(dialog).getByRole("button", { name: "启用自动化" });
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("disables the authorization parameters that a re-enable cannot change", async () => {
    const user = userEvent.setup();
    stubFetch("terminated");
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-terminated-notice");

    // 重新开启按原授权进行（payload 与终止记录逐字段相等）——本次表单的
    // 参数改动不生效，置灰避免用户以为会生效。
    expect(
      within(dialog).getByRole("checkbox", { name: "包含贯通/集成测试 Work Item" }),
    ).toBeDisabled();
    expect(
      within(dialog).getByRole("checkbox", { name: "包含 E2E 测试 Work Item" }),
    ).toBeDisabled();
    expect(
      within(dialog).getByRole("checkbox", { name: "强制前后端拆分" }),
    ).toBeDisabled();
    // 生成模式仍可切回手动（手动链路不经 enrollment）。
    expect(within(dialog).getByRole("radio", { name: "手动" })).toBeEnabled();
  });

  it("shows no termination notice when the issue has no enrollment", async () => {
    const user = userEvent.setup();
    stubFetch("none");
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-target-projection");

    await waitFor(() =>
      expect(
        within(dialog).queryByTestId("automation-terminated-notice"),
      ).toBeNull(),
    );
    const submit = within(dialog).getByRole("button", { name: "启用自动化" });
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("keeps the submit available when the enrollment projection cannot be read", async () => {
    const user = userEvent.setup();
    stubFetch("error");
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-target-projection");

    // 读取失败只是少一句说明：入口不被阻断，也不编造终止状态。
    await waitFor(() =>
      expect(
        within(dialog).queryByTestId("automation-terminated-notice"),
      ).toBeNull(),
    );
    const submit = within(dialog).getByRole("button", { name: "启用自动化" });
    await waitFor(() => expect(submit).toBeEnabled());
  });
});
