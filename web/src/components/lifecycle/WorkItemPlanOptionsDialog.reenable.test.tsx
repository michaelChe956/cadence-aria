import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { ApiRequestError } from "../../api/client";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { providerHealthSnapshot } from "../../state/provider-availability-test-fixtures";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";
import { carrierTargetFromEnrollmentTarget } from "./automation-reenable";

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

// 新投影只出逻辑代码库两变体（REQ-ROUTE-C5-TARGET 后端侧已下线单仓载体）。
const LOGICAL_TARGET = {
  kind: "logical_codebase",
  logical_codebase_id: "11111111-1111-1111-1111-111111111111",
  logical_repository_id: "00000000-0000-0000-0000-000000000001",
} as const;

/** legacy 只读载体：老单仓 enrollment 记录仍带这个 target（存量数据不丢）。 */
const LEGACY_SINGLE_REPOSITORY_TARGET = {
  kind: "single_repository",
  repository_id: "00000000-0000-0000-0000-000000000001",
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

function terminatedEnrollment(target: unknown = LOGICAL_TARGET): unknown {
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
    target,
    prepare_intent_id: "prepare_intent_0001",
    plan_id: null,
    session_id: null,
    created_at: "2026-10-16T00:00:00Z",
    updated_at: "2026-10-20T00:00:00Z",
    terminated_at: "2026-10-20T00:00:00Z",
  };
}

/** enrollment 投影按用例给定：null（无记录）、terminated 记录或读取失败。
 * `terminatedTarget` 可覆盖终止记录的载体（legacy 单仓载体场景）。 */
function stubFetch(
  enrollment: "none" | "terminated" | "error",
  terminatedTarget?: unknown,
): Mock {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/automation-target")) {
      return jsonResponse(targetResponse());
    }
    if (url.includes("/automation-enrollment")) {
      if (enrollment === "terminated") {
        return jsonResponse(terminatedEnrollment(terminatedTarget));
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
  useProviderAvailabilityStore.getState().reset();
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
    // REQ-WIGA-01 修订：工作项组生成路径不提供「手动」——单选项「自动化」保留可用。
    expect(
      within(dialog).queryByRole("radio", { name: "手动" }),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).getByRole("radio", { name: "自动化" }),
    ).toBeEnabled();
    // provider 是契约允许更换的（一次提交完成换代）——选择器保持可用。
    expect(within(dialog).getByLabelText("Author Provider")).toBeEnabled();
    expect(within(dialog).getByLabelText("Reviewer Provider")).toBeEnabled();
  });

  it("shows the provider that is actually in effect on the terminated enrollment", async () => {
    const user = userEvent.setup();
    stubFetch("terminated");
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    const providers = await within(dialog).findByTestId(
      "automation-effective-providers",
    );

    // 「实际会用的 provider」取自 enrollment 的生效 options（不是表单里
    // 未生效的值）——杜绝静默陷阱。
    expect(providers).toHaveTextContent("当前生效");
    expect(providers).toHaveTextContent("kimi_code");
    expect(providers).toHaveTextContent("pi");
  });

  it("states the provider that will be applied when the user picks another one", async () => {
    const user = userEvent.setup();
    stubFetch("terminated");
    useProviderAvailabilityStore.setState({
      loadStatus: "loaded",
      snapshot: providerHealthSnapshot({ codex: true, pi: true, kimi_code: true }),
    });
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-effective-providers");

    await user.selectOptions(
      within(dialog).getByLabelText("Author Provider"),
      "codex",
    );

    const providers = within(dialog).getByTestId(
      "automation-effective-providers",
    );
    // 当前生效仍是终止记录里的取值（如实展示），本次选择的 provider 另行说明。
    expect(providers).toHaveTextContent("kimi_code");
    expect(providers).toHaveTextContent("本次将应用");
    expect(providers).toHaveTextContent("codex");
    // REQ-WIGA-SWAP-01：换 provider 一次完成——文案 MUST 说明会先删除现有
    // 工作项组，MUST NOT 再说「重新开启后关闭再开启」（塌缩后是假话）。
    expect(providers).toHaveTextContent("换 provider 会先删除现有工作项组");
    expect(providers).not.toHaveTextContent("重新开启后关闭再开启生效");
  });

  it("shows the machine code on a secondary line, never in the main message", async () => {
    const user = userEvent.setup();
    stubFetch("none");
    // REQ-WIGA-SWAP-02：重新开启路径的包装把中文成品文案放主文案、机器码与
    // 原始 detail 降级为次要信息——两行都在，码 MUST NOT 充当主文案。
    const failure = new ApiRequestError({
      code: "automation_enrollment_revision_stale",
      message: "平台的授权记录刚发生变化，本次操作没有生效。等当前生成停下来再点一次（会自动重试）。",
      details: {},
    });
    const dialog = renderDialog(
      vi.fn(async () => {
        throw failure;
      }),
    );

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-target-projection");
    await user.click(
      within(dialog).getByRole("button", { name: "启用自动化" }),
    );

    // 主文案行：中文成品文案，不含机器码。
    const main = await within(dialog).findByTestId("submit-error-message");
    expect(main).toHaveTextContent(
      "平台的授权记录刚发生变化，本次操作没有生效。等当前生成停下来再点一次（会自动重试）。",
    );
    expect(main).not.toHaveTextContent("automation_enrollment_revision_stale");

    // 次要行：机器码这一行确实渲染出来，且是弱化的小号文本。
    const code = within(dialog).getByTestId("submit-error-code");
    expect(code).toHaveTextContent("automation_enrollment_revision_stale");
    expect(code).toHaveClass("text-xs");
    expect(code).not.toBe(main);
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

// 发起侧载体收窄（方案 A，REQ-ROUTE-C5-TARGET 前端侧）：legacy 单仓载体
// **只读保留**（老记录仍显示、不退化），但**不得再发起**重新开启／提交——
// 禁用提交并提示「需重建为逻辑代码库授权」。
describe("WorkItemPlanOptionsDialog：legacy 单仓载体的重新开启", () => {
  it("reenable_disabled_for_legacy_single_repository_binding", async () => {
    const user = userEvent.setup();
    // 终止记录的载体是 legacy 单仓投影。
    stubFetch("terminated", LEGACY_SINGLE_REPOSITORY_TARGET);
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    // legacy 只读渲染保留：终止记录仍显示物理仓身份。
    const projection = await within(dialog).findByTestId(
      "automation-target-projection",
    );
    expect(projection).toHaveTextContent("single_repository");
    expect(projection.textContent).not.toContain("undefined");

    // 发起侧收窄：重新开启被禁用，并给出可执行的下一步。
    const notice = await within(dialog).findByTestId(
      "automation-legacy-carrier-notice",
    );
    expect(notice).toHaveTextContent("需重建为逻辑代码库授权");
    const submit = within(dialog).getByRole("button", { name: "启用自动化" });
    await waitFor(() => expect(submit).toBeDisabled());
  });

  it("keeps the re-enable available for both logical-codebase carrier variants", async () => {
    const user = userEvent.setup();
    stubFetch("terminated");
    const dialog = renderDialog();

    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-terminated-notice");

    // LC 载体正常可重启：不被 legacy 阻断，也不误报提示。
    expect(
      within(dialog).queryByTestId("automation-legacy-carrier-notice"),
    ).toBeNull();
    const submit = within(dialog).getByRole("button", { name: "启用自动化" });
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("carrier_target_from_legacy_returns_null", () => {
    // legacy 单仓载体 → null（不可作为命令载体）。
    expect(
      carrierTargetFromEnrollmentTarget(LEGACY_SINGLE_REPOSITORY_TARGET),
    ).toBeNull();
    // LC 两变体原样透传。
    expect(carrierTargetFromEnrollmentTarget(LOGICAL_TARGET)).toEqual({
      kind: "logical_codebase",
      logical_codebase_id: "11111111-1111-1111-1111-111111111111",
      logical_repository_id: "00000000-0000-0000-0000-000000000001",
    });
    expect(
      carrierTargetFromEnrollmentTarget({
        kind: "logical_codebase_members",
        logical_codebase_id: "11111111-1111-1111-1111-111111111111",
        logical_repository_ids: ["00000000-0000-0000-0000-000000000001"],
      }),
    ).toEqual({
      kind: "logical_codebase_members",
      logical_codebase_id: "11111111-1111-1111-1111-111111111111",
      logical_repository_ids: ["00000000-0000-0000-0000-000000000001"],
    });
  });
});
