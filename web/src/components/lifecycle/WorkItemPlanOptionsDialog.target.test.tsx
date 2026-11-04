import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "../../api/client";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";
import { carrierTargetFromEnrollmentTarget } from "./automation-reenable";

// C5 Task 7：automatic 模式的 target projection 生命周期——打开自动模式先取
// automation-target 投影（展示 target.kind 与双级身份），loading/error 禁止
// stale automatic submit 且提供重试；manual 模式不请求 target；role-chain
// 预检错误逐角色渲染 role/provider/reason，ApiRequestError.details 不被
// 包装为普通 Error。

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

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

function targetResponse(
  enrollment_target: unknown = LOGICAL_TARGET,
): unknown {
  return {
    enrollment_target,
    resolved_options: {
      author_provider: "claude_code",
      reviewer_provider: "codex",
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      plan_options: {
        include_integration_tests: true,
        include_e2e_tests: false,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
      },
    },
  };
}

type ConfirmHandler = (
  options: WorkItemPlanOptionsFormValue,
) => Promise<void> | void;

function renderDialog(
  overrides: Partial<{
    onConfirm: ConfirmHandler;
    scope: { projectId: string; issueId: string } | null;
  }> = {},
) {
  return render(
    <WorkItemPlanOptionsDialog
      defaultOptions={BASE_OPTIONS}
      automationAvailable
      automationTargetScope={overrides.scope ?? SCOPE}
      onConfirm={
        overrides.onConfirm ?? (vi.fn(async () => undefined) as ConfirmHandler)
      }
      onClose={vi.fn()}
    />,
  );
}

const BASE_OPTIONS: WorkItemPlanOptionsFormValue = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
  automation_mode: "manual",
};

const SCOPE = { projectId: "project_0001", issueId: "issue_0001" } as const;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("WorkItemPlanOptionsDialog automation target projection", () => {
  it("fetches the target projection when automatic is selected and gates submit until ready", async () => {
    const user = userEvent.setup();
    const pending = deferred<Response>();
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        pending.promise,
    );
    vi.stubGlobal("fetch", fetchMock);
    renderDialog();

    // manual 缺省不请求 target。
    await user.click(
      screen.getByRole("button", { name: "创建并打开 Workspace" }),
    );
    expect(
      fetchMock.mock.calls.some(([url]) =>
        String(url).includes("/automation-target"),
      ),
    ).toBe(false);

    // 打开对话框重新进入 automatic：radio 切换触发 GET automation-target。
    const dialog = screen.getByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([url]) =>
          String(url).includes("/automation-target"),
        ),
      ).toBe(true),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/automation-target",
      expect.anything(),
    );

    // loading 中禁止 stale automatic submit。
    const submit = within(dialog).getByRole("button", {
      name: "启用自动化",
    });
    expect(submit).toBeDisabled();

    pending.resolve(jsonResponse(targetResponse()) as Response);
    const projection = await within(dialog).findByTestId(
      "automation-target-projection",
    );
    expect(projection).toHaveTextContent("logical_codebase");
    expect(projection).toHaveTextContent("11111111-1111-1111-1111-111111111111");
    expect(projection).toHaveTextContent(
      "00000000-0000-0000-0000-000000000001",
    );
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("legacy_single_repository_projection_renders_readonly", async () => {
    const user = userEvent.setup();
    const fetchMock = vi.fn(async () =>
      jsonResponse(
        targetResponse({
          kind: "single_repository",
          repository_id: "repo_physical_c5",
        }),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    renderDialog();

    const dialog = screen.getByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    const projection = await within(dialog).findByTestId(
      "automation-target-projection",
    );
    // legacy 只读渲染保留：老单仓记录仍显示物理仓身份，**不退化为「target
    // 未知」**（存量数据不丢信息）。
    expect(projection).toHaveTextContent("single_repository");
    expect(projection).toHaveTextContent("repo_physical_c5");
    expect(projection.textContent).not.toContain("undefined");

    // 发起侧收窄：单仓载体不得再作为命令载体 ⇒ 提交禁用并给出提示。
    const notice = await within(dialog).findByTestId(
      "automation-legacy-carrier-notice",
    );
    expect(notice).toHaveTextContent("需重建为逻辑代码库授权");
    const submit = within(dialog).getByRole("button", {
      name: "启用自动化",
    });
    await waitFor(() => expect(submit).toBeDisabled());
  });

  it("keeps automatic submit disabled with a retryable error when the target GET fails", async () => {
    const user = userEvent.setup();
    // 按 URL 分派：自动模式还会读一次 enrollment 只读投影（terminated 事实
    // 展示用），不能让它吃掉 target 的失败/成功序列。
    let targetCalls = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/automation-enrollment")) {
        return jsonResponse(null);
      }
      targetCalls += 1;
      if (targetCalls === 1) {
        return jsonResponse(
          {
            code: "automation_enrollment_invalid_scope",
            message: "automation target requires exactly one logical repository",
            details: {},
          },
          422,
        );
      }
      return jsonResponse(targetResponse());
    });
    vi.stubGlobal("fetch", fetchMock);
    renderDialog();

    const dialog = screen.getByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent(/自动化目标加载失败/);
    const submit = within(dialog).getByRole("button", {
      name: "启用自动化",
    });
    expect(submit).toBeDisabled();

    // GET 失败可重试；重试成功后恢复提交。
    await user.click(within(dialog).getByTestId("automation-target-retry"));
    const projection = await within(dialog).findByTestId(
      "automation-target-projection",
    );
    expect(projection).toHaveTextContent("logical_codebase");
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("does not request the target in manual mode and manual submit stays available after a target failure", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn(async () => undefined);
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        jsonResponse(
          { code: "web_client_error", message: "target GET failed", details: {} },
          500,
        ),
      );
    vi.stubGlobal("fetch", fetchMock);
    renderDialog({ onConfirm });

    const dialog = screen.getByRole("dialog", { name: "Work Item Plan 配置" });
    // automatic GET 失败后切回 manual：不因 target 失败阻断人工链路。
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByRole("alert");
    await user.click(within(dialog).getByRole("radio", { name: "手动" }));
    const submit = within(dialog).getByRole("button", {
      name: "创建并打开 Workspace",
    });
    await waitFor(() => expect(submit).toBeEnabled());
    await user.click(submit);
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(1));
    expect(
      fetchMock.mock.calls.filter(([url]) =>
        String(url).includes("/automation-target"),
      ),
    ).toHaveLength(1);
  });

  it("renders every role-chain violation from ApiRequestError details without wrapping it", async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse(targetResponse())),
    );
    const roleChainError = new ApiRequestError({
      code: "automation_role_chain_unsupported",
      message: "automation role chain is not supported for this carrier",
      details: {
        violations: [
          {
            role: "plan_author",
            provider: "pi",
            reason_code: "provider_unsupported_for_gateway_launch",
          },
          {
            role: "internal_reviewer",
            provider: "kimi_code",
            reason_code: "gateway_route_blocked",
          },
        ],
      },
    });
    const onConfirm = vi.fn(() => Promise.reject(roleChainError));
    renderDialog({ onConfirm });

    const dialog = screen.getByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await within(dialog).findByTestId("automation-target-projection");
    await user.click(
      within(dialog).getByRole("button", { name: "启用自动化" }),
    );

    const alert = await within(dialog).findByRole("alert");
    expect(
      within(alert).getAllByTestId("role-chain-violation"),
    ).toHaveLength(2);
    expect(alert).toHaveTextContent("plan_author");
    expect(alert).toHaveTextContent("pi");
    expect(alert).toHaveTextContent(
      "provider_unsupported_for_gateway_launch",
    );
    expect(alert).toHaveTextContent("internal_reviewer");
    expect(alert).toHaveTextContent("kimi_code");
    expect(alert).toHaveTextContent("gateway_route_blocked");
    expect(alert).toHaveTextContent(/更换 provider 配置或更换目标/);
  });
});

// C5：多成员逻辑代码库（logical_codebase_members）target 的展示身份。
// 后端 EnrollmentTarget 有三个变体，投影回传第三个变体时前端必须渲染成员
// 数量，不得打印 undefined/null/[object Object]。
describe("WorkItemPlanOptionsDialog enrollment target label variants", () => {
  const LC_ID = "11111111-1111-1111-1111-111111111111";

  async function renderProjection(target: unknown): Promise<HTMLElement> {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse(targetResponse(target))),
    );
    renderDialog();
    const dialog = screen.getByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    return within(dialog).findByTestId("automation-target-projection");
  }

  it("labels a single-repository target", async () => {
    const projection = await renderProjection({
      kind: "single_repository",
      repository_id: "repo_physical_c5",
    });
    // legacy 只读渲染保留：仍显示「单仓 <物理仓>」，不退化为「target 未知」。
    expect(projection).toHaveTextContent("single_repository repo_physical_c5");
    expect(projection.textContent).not.toContain("undefined");
  });

  it("labels a single-member logical-codebase target", async () => {
    const projection = await renderProjection({
      kind: "logical_codebase",
      logical_codebase_id: LC_ID,
      logical_repository_id: "00000000-0000-0000-0000-000000000001",
    });
    expect(projection).toHaveTextContent(
      `logical_codebase ${LC_ID}/00000000-0000-0000-0000-000000000001`,
    );
    expect(projection.textContent).not.toContain("undefined");
  });

  it("target_label_renders_logical_codebase_members", async () => {
    const projection = await renderProjection({
      kind: "logical_codebase_members",
      logical_codebase_id: LC_ID,
      logical_repository_ids: [
        "00000000-0000-0000-0000-000000000001",
        "00000000-0000-0000-0000-000000000002",
        "00000000-0000-0000-0000-000000000003",
      ],
    });
    // 多成员 LC 的展示身份：逻辑代码库 id + 成员数量（N 个成员）。
    expect(projection).toHaveTextContent(
      `logical_codebase ${LC_ID}[3 个成员]`,
    );
    expect(projection.textContent).not.toContain("undefined");
    expect(projection.textContent).not.toContain("null");
  });

  it("labels a multi-member logical-codebase target without members", async () => {
    const projection = await renderProjection({
      kind: "logical_codebase_members",
      logical_codebase_id: LC_ID,
      logical_repository_ids: [],
    });
    expect(projection).toHaveTextContent(`logical_codebase ${LC_ID}[无成员]`);
    expect(projection.textContent).not.toContain("undefined");
  });

  it("never renders undefined for a target with missing fields", async () => {
    const projection = await renderProjection({
      kind: "logical_codebase_members",
    });
    expect(projection.textContent).not.toContain("undefined");
    expect(projection.textContent).not.toContain("null");
    expect(projection.textContent).not.toContain("[object Object]");
    expect(projection).toHaveTextContent("—");
  });
});

// 发起侧载体收窄（方案 A，REQ-ROUTE-C5-TARGET 前端侧）：命令侧（Enable /
// Rebind payload）的 target 只有逻辑代码库两变体——投影侧第三变体
// `single_repository` 仅 legacy 只读，守卫把它收敛为 null（不可作载体）。
// 断言**限定在守卫与命令构造处**（不扫整个组件的字面量——注释与用例名里也
// 会出现该字样）。
describe("命令侧载体守卫（automation-reenable）", () => {
  const LC_ID = "11111111-1111-1111-1111-111111111111";

  it("no_single_repository_in_carrier_commands", () => {
    // legacy 单仓载体 → null：不得作为命令载体（调用方禁用提交并提示）。
    expect(
      carrierTargetFromEnrollmentTarget({
        kind: "single_repository",
        repository_id: "repo_physical_c5",
      }),
    ).toBeNull();

    // 逻辑代码库两变体原样透传，且**任何情况下都不产出单仓载体**。
    const carriers = [
      carrierTargetFromEnrollmentTarget({
        kind: "logical_codebase",
        logical_codebase_id: LC_ID,
        logical_repository_id: "00000000-0000-0000-0000-000000000001",
      }),
      carrierTargetFromEnrollmentTarget({
        kind: "logical_codebase_members",
        logical_codebase_id: LC_ID,
        logical_repository_ids: ["00000000-0000-0000-0000-000000000001"],
      }),
    ];
    for (const carrier of carriers) {
      expect(carrier).not.toBeNull();
      expect(carrier?.kind).not.toBe("single_repository");
    }
    expect(carriers[0]).toEqual({
      kind: "logical_codebase",
      logical_codebase_id: LC_ID,
      logical_repository_id: "00000000-0000-0000-0000-000000000001",
    });
    expect(carriers[1]).toEqual({
      kind: "logical_codebase_members",
      logical_codebase_id: LC_ID,
      logical_repository_ids: ["00000000-0000-0000-0000-000000000001"],
    });
  });
});
