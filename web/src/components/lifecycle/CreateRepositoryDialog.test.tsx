import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ApiError,
  CreateRepositoryResponse,
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
  RepositoryInitializationOperationSnapshot,
  RepositoryInitializationStep,
  RepositoryInitializationStepId,
  RepositoryInitializationStepStatus,
} from "../../api/types";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { CreateRepositoryDialog } from "./CreateRepositoryDialog";

function providerEntry(
  provider: RealProviderName,
  available: boolean,
): ProviderHealthEntry {
  return {
    provider,
    display_name: provider === "claude_code" ? "Claude Code" : "Codex",
    available,
    version: available ? "1.0.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : `${provider} 未安装`,
    checked_at: "2026-07-14T00:00:00Z",
    install_hint: `安装 ${provider}`,
  };
}

function setProviderHealth(
  claudeAvailable: boolean,
  codexAvailable: boolean,
  overrides: Partial<ProviderHealthResponse> = {},
) {
  const snapshot: ProviderHealthResponse = {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: !claudeAvailable && !codexAvailable,
    test_provider_enabled: false,
    providers: [
      providerEntry("claude_code", claudeAvailable),
      providerEntry("codex", codexAvailable),
    ],
    ...overrides,
  };
  useProviderAvailabilityStore.setState({
    snapshot,
    loadStatus: "loaded",
    realWorkflowBlocked: snapshot.real_workflow_blocked,
    testProviderEnabled: snapshot.test_provider_enabled,
  });
}

function createResponse(): CreateRepositoryResponse {
  return {
    repository: {
      repository_id: "repository_0001",
      project_id: "project_0001",
      name: "Aria",
      path: "/work/aria",
      repo_hash: "hash",
      runtime_root: "/work/aria/.aria/runtime",
      default_policy_preset: "manual-write",
      default_provider_mode: "codex",
      created_at: "2026-07-14T00:00:00Z",
      updated_at: "2026-07-14T00:01:00Z",
    },
    initialization: {
      source: "offline",
      commands: [
        { index: 1, command: "/cadence-init:rule-config", status: "completed" },
        { index: 2, command: "/cadence-init:pre-check", status: "completed" },
      ],
      warnings: ["cadence_skills_conflict:<path>"],
      changed_paths: [".claude/rules/project.md"],
      completed_at: "2026-07-14T00:01:00Z",
    },
  };
}

function steps(
  states: RepositoryInitializationStepStatus[],
): RepositoryInitializationStep[] {
  const stepIds: RepositoryInitializationStepId[] = [
    "cadence_skills",
    "rule_config",
    "pre_check",
    "mcp_configuration",
    "project_rules_examples",
  ];
  return stepIds.map((step_id, index) => ({ step_id, status: states[index]! }));
}

function operation(
  overrides: Partial<RepositoryInitializationOperationSnapshot> = {},
): RepositoryInitializationOperationSnapshot {
  return {
    operation_id: "repository_initialization_0001",
    status: "running",
    steps: steps(["running", "pending", "pending", "pending", "pending"]),
    current_step: "cadence_skills",
    failed_step: null,
    result: null,
    error: null,
    created_at: "2026-07-22T00:00:00Z",
    updated_at: "2026-07-22T00:00:00Z",
    completed_at: null,
    ...overrides,
  };
}

function completedOperation(
  overrides: Partial<RepositoryInitializationOperationSnapshot> = {},
): RepositoryInitializationOperationSnapshot {
  return operation({
    status: "completed",
    steps: steps([
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
    ]),
    current_step: null,
    result: createResponse(),
    completed_at: "2026-07-22T00:01:00Z",
    ...overrides,
  });
}

function failedOperation(
  overrides: Partial<RepositoryInitializationOperationSnapshot> = {},
): RepositoryInitializationOperationSnapshot {
  return operation({
    status: "failed",
    steps: steps(["completed", "failed", "pending", "pending", "pending"]),
    current_step: null,
    failed_step: "rule_config",
    error: {
      code: "repository_init_command_failed",
      message: "repository registration failed",
      details: {
        action: "修复权限后重试",
      },
    },
    ...overrides,
  });
}

async function fillRequiredFields(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText("代码库名称"), "Aria");
  await user.type(screen.getByLabelText("本地路径"), "/work/aria");
}

async function submitRequiredFieldsWithFakeTimers() {
  fireEvent.change(screen.getByLabelText("代码库名称"), {
    target: { value: "Aria" },
  });
  fireEvent.change(screen.getByLabelText("本地路径"), {
    target: { value: "/work/aria" },
  });
  fireEvent.submit(screen.getByRole("dialog", { name: "添加代码库" }));
  await flushAsyncWork();
}

async function flushAsyncWork() {
  await act(async () => {
    await Promise.resolve();
  });
}

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  useProviderAvailabilityStore.getState().reset();
});

describe("CreateRepositoryDialog", () => {
  it("shows five server-driven steps and disables close while running", async () => {
    setProviderHealth(true, true);
    const user = userEvent.setup();
    const running = operation();
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(running)}
        onFetchOperation={vi.fn().mockResolvedValue(running)}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    await fillRequiredFields(user);
    await user.click(screen.getByRole("button", { name: "添加代码库" }));

    const dialog = await screen.findByRole("dialog", { name: "添加代码库" });
    const stepList = within(dialog).getByRole("list", { name: "初始化步骤" });
    expect(within(stepList).getAllByRole("listitem")).toHaveLength(5);
    expect(stepList).toHaveTextContent("准备 Cadence Skills");
    expect(stepList).toHaveTextContent("执行预检查");
    expect(stepList).toHaveTextContent("配置规则");
    expect(stepList).toHaveTextContent("配置 MCP");
    expect(stepList).toHaveTextContent("生成项目规则示例");
    expect(within(dialog).getByRole("status")).toHaveTextContent("已完成 0 / 5");
    expect(dialog).toHaveTextContent("正在初始化，请保持此窗口打开");
    expect(dialog).toHaveFocus();
    expect(within(dialog).getByRole("button", { name: "关闭" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "取消" })).toBeDisabled();
  });

  it("polls through real snapshots and stops at completed", async () => {
    vi.useFakeTimers();
    setProviderHealth(true, true);
    const firstPoll = operation({
      steps: steps(["completed", "running", "pending", "pending", "pending"]),
      current_step: "rule_config",
    });
    const completed = completedOperation();
    const onFetchOperation = vi
      .fn()
      .mockResolvedValueOnce(firstPoll)
      .mockResolvedValueOnce(completed);
    const onInitializationCompleted = vi.fn();
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(operation())}
        onFetchOperation={onFetchOperation}
        onInitializationCompleted={onInitializationCompleted}
        onClose={vi.fn()}
      />,
    );

    await submitRequiredFieldsWithFakeTimers();

    expect(onFetchOperation).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("status")).toHaveTextContent(
      "正在执行：配置规则。已完成 1 / 5。",
    );
    const stepList = screen.getByRole("list", { name: "初始化步骤" });
    expect(within(stepList).getByText("准备 Cadence Skills").closest("li")).toHaveTextContent(
      "已完成",
    );
    expect(within(stepList).getByText("配置规则").closest("li")).toHaveTextContent(
      "正在执行",
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    await flushAsyncWork();

    expect(onFetchOperation).toHaveBeenCalledTimes(2);
    expect(onInitializationCompleted).toHaveBeenCalledTimes(1);
    expect(onInitializationCompleted).toHaveBeenCalledWith(completed.result);
    expect(screen.getByText("代码库初始化完成")).toBeInTheDocument();
    expect(screen.getByText("offline")).toBeInTheDocument();
    expect(screen.getByText(".claude/rules/project.md")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "完成" })).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(onFetchOperation).toHaveBeenCalledTimes(2);
  });

  it("shows a local error and lets the user refill when a completed operation has no result", async () => {
    setProviderHealth(true, true);
    const user = userEvent.setup();
    const onInitializationCompleted = vi.fn();
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(
          completedOperation({ result: null }),
        )}
        onFetchOperation={vi.fn()}
        onInitializationCompleted={onInitializationCompleted}
        onClose={vi.fn()}
      />,
    );

    await fillRequiredFields(user);
    await user.click(screen.getByRole("button", { name: "添加代码库" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "代码库初始化已完成，但服务未返回初始化结果",
    );
    expect(screen.queryByText("代码库初始化完成")).not.toBeInTheDocument();
    expect(onInitializationCompleted).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "重新填写" }));
    expect(screen.getByLabelText("代码库名称")).toHaveValue("Aria");
    expect(screen.getByLabelText("代码库名称")).toHaveFocus();
  });

  it("moves focus to the preserved name field after refilling a failed operation", async () => {
    setProviderHealth(true, true);
    const user = userEvent.setup();
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(failedOperation())}
        onFetchOperation={vi.fn()}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    await fillRequiredFields(user);
    await user.click(screen.getByRole("button", { name: "添加代码库" }));
    await user.click(screen.getByRole("button", { name: "重新填写" }));

    expect(screen.getByLabelText("代码库名称")).toHaveValue("Aria");
    expect(screen.getByLabelText("代码库名称")).toHaveFocus();
  });

  it("keeps last known steps when one poll fails and retries", async () => {
    vi.useFakeTimers();
    setProviderHealth(true, true);
    const retrySnapshot = operation({
      steps: steps(["completed", "running", "pending", "pending", "pending"]),
      current_step: "rule_config",
    });
    const onFetchOperation = vi
      .fn()
      .mockRejectedValueOnce("temporary polling failure")
      .mockResolvedValueOnce(retrySnapshot);
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(operation())}
        onFetchOperation={onFetchOperation}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    await submitRequiredFieldsWithFakeTimers();

    const stepList = screen.getByRole("list", { name: "初始化步骤" });
    expect(screen.getByText("正在重试获取初始化状态")).toBeInTheDocument();
    expect(within(stepList).getByText("准备 Cadence Skills").closest("li")).toHaveTextContent(
      "正在执行",
    );
    expect(within(stepList).queryByText("失败")).not.toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    await flushAsyncWork();

    expect(onFetchOperation).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("status")).toHaveTextContent(
      "正在执行：配置规则。已完成 1 / 5。",
    );
    expect(within(stepList).getByText("准备 Cadence Skills").closest("li")).toHaveTextContent(
      "已完成",
    );
    expect(within(stepList).getByText("配置规则").closest("li")).toHaveTextContent(
      "正在执行",
    );
  });

  it("renders failed step and lets the user submit a new operation", async () => {
    setProviderHealth(true, true);
    const user = userEvent.setup();
    const error: ApiError = {
      code: "repository_init_command_failed",
      message: "repository registration failed",
      details: {
        reason: "初始化命令执行失败",
        stage: "repository_init_command",
        provider: "claude_code",
        command: "/rule-config",
        reason_code: "repository_init_command_failed",
        stderr_summary: "permission denied",
        changed_paths: [
          ".claude/rules/project.md",
          "cadence/project-rules/README.md",
        ],
        retryable: true,
        action: "修复权限后重试",
      },
    };
    const failed = failedOperation({
      error,
    });
    const onCreate = vi
      .fn()
      .mockResolvedValueOnce(failed)
      .mockResolvedValueOnce(operation());
    render(
      <CreateRepositoryDialog
        onCreate={onCreate}
        onFetchOperation={vi.fn().mockResolvedValue(operation())}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    await fillRequiredFields(user);
    await user.click(screen.getByRole("button", { name: "添加代码库" }));

    const stepList = await screen.findByRole("list", { name: "初始化步骤" });
    const failedStep = within(stepList).getByText("配置规则").closest("li");
    expect(failedStep).toHaveTextContent("失败");
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("repository registration failed");
    expect(alert).toHaveTextContent("changed_paths");
    expect(alert).toHaveTextContent(".claude/rules/project.md");
    expect(alert).toHaveTextContent("cadence/project-rules/README.md");
    expect(alert).toHaveTextContent("retryable");
    expect(alert).toHaveTextContent("true");
    expect(alert).toHaveTextContent("action");
    expect(alert).toHaveTextContent("修复权限后重试");

    await user.click(screen.getByRole("button", { name: "重新填写" }));

    expect(screen.getByLabelText("代码库名称")).toHaveValue("Aria");
    expect(screen.getByLabelText("本地路径")).toHaveValue("/work/aria");
    expect(screen.getByLabelText("Policy")).toHaveValue("manual-write");
    expect(screen.getByLabelText("Provider")).toHaveValue("codex");
    await user.click(screen.getByRole("button", { name: "添加代码库" }));
    expect(onCreate).toHaveBeenCalledTimes(2);
  });

  it("cleans up polling after unmount", async () => {
    vi.useFakeTimers();
    setProviderHealth(true, true);
    const running = operation();
    const onFetchOperation = vi.fn().mockResolvedValue(running);
    const { unmount } = render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(running)}
        onFetchOperation={onFetchOperation}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    await submitRequiredFieldsWithFakeTimers();
    const callsBeforeUnmount = onFetchOperation.mock.calls.length;

    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });

    expect(onFetchOperation).toHaveBeenCalledTimes(callsBeforeUnmount);
  });

  it("blocks initialization when Claude Code is unavailable even if Codex is available", async () => {
    setProviderHealth(false, true);
    const user = userEvent.setup();
    const onCreate = vi.fn();
    render(
      <CreateRepositoryDialog
        onCreate={onCreate}
        onFetchOperation={vi.fn()}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    const provider = screen.getByLabelText("Provider");
    expect(within(provider).getByRole("option", { name: "Claude Code" })).toBeDisabled();
    expect(within(provider).getByRole("option", { name: "Codex" })).toBeEnabled();
    expect(within(provider).queryByRole("option", { name: "Fake" })).not.toBeInTheDocument();
    expect(screen.getByText(/代码库初始化固定要求 Claude Code/u)).toBeInTheDocument();
    expect(screen.getByText("安装 claude_code")).toBeInTheDocument();

    await fillRequiredFields(user);
    expect(screen.getByRole("button", { name: "添加代码库" })).toBeDisabled();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("deduplicates submit while preserving the selected default Provider meaning", async () => {
    setProviderHealth(true, true);
    const user = userEvent.setup();
    let resolveCreate!: (snapshot: RepositoryInitializationOperationSnapshot) => void;
    const onCreate = vi.fn(
      () =>
        new Promise<RepositoryInitializationOperationSnapshot>((resolve) => {
          resolveCreate = resolve;
        }),
    );
    render(
      <CreateRepositoryDialog
        onCreate={onCreate}
        onFetchOperation={vi.fn()}
        onInitializationCompleted={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    await fillRequiredFields(user);

    const submit = screen.getByRole("button", { name: "添加代码库" });
    await user.dblClick(submit);

    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith({
      name: "Aria",
      path: "/work/aria",
      default_policy_preset: "manual-write",
      default_provider_mode: "codex",
    });
    expect(submit).toBeDisabled();

    resolveCreate(completedOperation());
    expect(await screen.findByText("代码库初始化完成")).toBeInTheDocument();
  });
});
