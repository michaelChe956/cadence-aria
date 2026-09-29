import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "../../api/client";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";

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

  it("shows a single-repository target with its physical repository identity", async () => {
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
    expect(projection).toHaveTextContent("single_repository");
    expect(projection).toHaveTextContent("repo_physical_c5");
  });

  it("keeps automatic submit disabled with a retryable error when the target GET fails", async () => {
    const user = userEvent.setup();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        jsonResponse(
          {
            code: "automation_enrollment_invalid_scope",
            message: "automation target requires exactly one logical repository",
            details: {},
          },
          422,
        ),
      )
      .mockResolvedValueOnce(jsonResponse(targetResponse()));
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
