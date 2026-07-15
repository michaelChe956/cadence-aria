import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "../../api/client";
import type {
  CreateRepositoryResponse,
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
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
        { index: 1, command: "/cadence-init:pre-check", status: "completed" },
        { index: 2, command: "/cadence-init:rule-config", status: "completed" },
      ],
      warnings: ["cadence_skills_conflict:<path>"],
      changed_paths: [".claude/rules/project.md"],
      completed_at: "2026-07-14T00:01:00Z",
    },
  };
}

async function fillRequiredFields() {
  await userEvent.type(screen.getByLabelText("代码库名称"), "Aria");
  await userEvent.type(screen.getByLabelText("本地路径"), "/work/aria");
}

afterEach(() => {
  useProviderAvailabilityStore.getState().reset();
});

describe("CreateRepositoryDialog", () => {
  it("blocks initialization when Claude Code is unavailable even if Codex is available", async () => {
    setProviderHealth(false, true);
    const onCreate = vi.fn();
    render(<CreateRepositoryDialog onCreate={onCreate} onClose={vi.fn()} />);

    const provider = screen.getByLabelText("Provider");
    expect(within(provider).getByRole("option", { name: "Claude Code" })).toBeDisabled();
    expect(within(provider).getByRole("option", { name: "Codex" })).toBeEnabled();
    expect(within(provider).queryByRole("option", { name: "Fake" })).not.toBeInTheDocument();
    expect(screen.getByText(/代码库初始化固定要求 Claude Code/u)).toBeInTheDocument();
    expect(screen.getByText("安装 claude_code")).toBeInTheDocument();

    await fillRequiredFields();
    expect(screen.getByRole("button", { name: "添加代码库" })).toBeDisabled();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("deduplicates submit while preserving the selected default Provider meaning", async () => {
    setProviderHealth(true, true);
    let resolveCreate!: (response: CreateRepositoryResponse) => void;
    const onCreate = vi.fn(
      () =>
        new Promise<CreateRepositoryResponse>((resolve) => {
          resolveCreate = resolve;
        }),
    );
    render(<CreateRepositoryDialog onCreate={onCreate} onClose={vi.fn()} />);
    await fillRequiredFields();

    const submit = screen.getByRole("button", { name: "添加代码库" });
    await userEvent.dblClick(submit);

    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith({
      name: "Aria",
      path: "/work/aria",
      default_policy_preset: "manual-write",
      default_provider_mode: "codex",
    });
    expect(submit).toBeDisabled();

    resolveCreate(createResponse());
    expect(await screen.findByText("代码库初始化完成")).toBeInTheDocument();
  });

  it("keeps the dialog open and presents the complete initialization summary until confirmed", async () => {
    setProviderHealth(true, true);
    const onClose = vi.fn();
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockResolvedValue(createResponse())}
        onClose={onClose}
      />,
    );
    await fillRequiredFields();
    await userEvent.click(screen.getByRole("button", { name: "添加代码库" }));

    const dialog = await screen.findByRole("dialog", { name: "添加代码库" });
    expect(dialog).toHaveTextContent("代码库初始化完成");
    expect(dialog).toHaveTextContent("offline");
    expect(dialog).toHaveTextContent("cadence_skills_conflict:<path>");
    expect(dialog).toHaveTextContent("2026-07-14T00:01:00Z");
    expect(dialog).toHaveTextContent("/cadence-init:pre-check");
    expect(dialog).toHaveTextContent(".claude/rules/project.md");
    expect(onClose).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("renders structured registration error details without discarding repository changes", async () => {
    setProviderHealth(true, true);
    const error = new ApiRequestError({
      code: "repository_init_command_failed",
      message: "repository registration failed",
      details: {
        reason: "初始化命令执行失败",
        stage: "repository_init_command",
        provider: "claude_code",
        command: "/rule-config",
        reason_code: "repository_init_command_failed",
        stderr_summary: "permission denied",
        changed_paths: [".claude/rules/project.md", "cadence/project-rules/README.md"],
        retryable: true,
        action: "修复权限后重试",
      },
    });
    render(
      <CreateRepositoryDialog
        onCreate={vi.fn().mockRejectedValue(error)}
        onClose={vi.fn()}
      />,
    );
    await fillRequiredFields();
    await userEvent.click(screen.getByRole("button", { name: "添加代码库" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("repository registration failed");
    expect(alert).toHaveTextContent("初始化命令执行失败");
    expect(alert).toHaveTextContent("repository_init_command");
    expect(alert).toHaveTextContent("claude_code");
    expect(alert).toHaveTextContent("/rule-config");
    expect(alert).toHaveTextContent("repository_init_command_failed");
    expect(alert).toHaveTextContent("permission denied");
    expect(alert).toHaveTextContent(".claude/rules/project.md");
    expect(alert).toHaveTextContent("cadence/project-rules/README.md");
    expect(alert).toHaveTextContent("修复权限后重试");
    expect(alert).toHaveTextContent("目标代码库中的上述修改可能已保留，系统未执行破坏性回滚");
    expect(alert).toHaveTextContent("修复问题后可以重新提交");
    expect(alert).not.toHaveTextContent("目标代码库完全无变化");
  });
});
