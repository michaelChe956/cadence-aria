// Task 9：LC 仓管理抽屉内「知识库」卡片的测试。
// 测试名与按钮文案按 task-9-brief Step 1 逐字落位。
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi, type MockInstance } from "vitest";
import type { AggregateProviderSelectionDto } from "../../api/types";
import type {
  KnowledgeBaseOperationSnapshotDto,
  KnowledgeBaseStatusDto,
} from "../../api/types/knowledge-base";
import { KnowledgeBaseCard } from "./KnowledgeBaseCard";
import type { KnowledgeBaseCardProps } from "./KnowledgeBaseCard";

// 卡片内的更新弹窗会拉取变更包列表；把 API 客户端整体打桩，避免测试触网。
vi.mock("../../api/knowledge-base", () => ({
  listKnowledgeBaseChangePackages: vi.fn().mockResolvedValue([
    {
      id: "cp_0001",
      path: "changes/add-login.md",
      violation_count: 0,
      violations: [],
    },
  ]),
  createKnowledgeBaseChangePackage: vi
    .fn()
    .mockResolvedValue({ id: "cp_0002", path: "changes/new-feature.md" }),
}));

const statusDto = (
  initializationStatus: string | null,
  globalValidation: string | null = "passed",
): KnowledgeBaseStatusDto => ({
  exists: initializationStatus !== null,
  schema_version: initializationStatus !== null ? "4.0" : null,
  initialization_status: initializationStatus,
  global_validation: globalValidation,
  completed_stages: initializationStatus === "complete" ? ["bootstrap"] : [],
  skipped_stages: [],
  skills_available: true,
});

const providerSelection: AggregateProviderSelectionDto = {
  selected: "pi",
  candidates: [
    {
      provider: "pi",
      display_name: "Pi",
      cli_available: true,
      cli_reason_code: null,
      capability_state: "confirmed",
      skipped_reason: null,
      action_hint: "直接使用",
    },
    {
      provider: "kimi_code",
      display_name: "Kimi",
      cli_available: true,
      cli_reason_code: null,
      capability_state: "unverified",
      skipped_reason: "needs_verification",
      action_hint: "核验 Kimi",
    },
    {
      provider: "claude_code",
      display_name: "Claude Code",
      cli_available: false,
      cli_reason_code: "command_missing",
      capability_state: "not_installed",
      skipped_reason: "cli_unavailable",
      action_hint: "安装 Claude Code",
    },
  ],
  rule_version: "v1",
  decided_at: "2026-11-09T00:00:00Z",
};

const operationWithProviderSelection: KnowledgeBaseOperationSnapshotDto = {
  operation_id: "kb_op_0001",
  project_id: "project_0001",
  logical_codebase_id: "logical_codebase_0001",
  kind: "bootstrap",
  input: {
    idempotency_key: "idem-0001",
    provider: "pi",
    package_path: null,
    package_id: null,
  },
  status: "completed",
  steps: [
    {
      step_id: "generate",
      status: "completed",
      started_at: null,
      completed_at: null,
      output_summary: null,
    },
  ],
  provider_selection: providerSelection,
  error: null,
  created_at: "2026-11-09T00:00:00Z",
  started_at: null,
  completed_at: null,
};

function renderCard(overrides: Partial<KnowledgeBaseCardProps> = {}) {
  const props: KnowledgeBaseCardProps = {
    projectId: "project_0001",
    logicalCodebaseId: "logical_codebase_0001",
    status: null,
    operation: null,
    busy: false,
    preflightFailure: null,
    onStart: vi.fn(),
    onCancel: vi.fn(),
    onOpenDetail: vi.fn(),
    onCreateBootstrapInputs: vi.fn(),
    onDismissPreflightFailure: vi.fn(),
    ...overrides,
  };
  render(<KnowledgeBaseCard {...props} />);
  return props;
}

// 只还原 window.confirm 的 spy：vi.restoreAllMocks 会把 vi.mock 工厂里
// vi.fn().mockResolvedValue(...) 的实现一并清掉，导致弹窗拉取返回 undefined。
let confirmSpy: MockInstance<typeof window.confirm> | null = null;

afterEach(() => {
  confirmSpy?.mockRestore();
  confirmSpy = null;
});

describe("KnowledgeBaseCard", () => {
  it("renders_three_buttons", () => {
    renderCard();
    expect(
      screen.getByRole("button", { name: "知识库初始化" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "知识库更新" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "查看知识库详细内容" }),
    ).toBeInTheDocument();
  });

  it("update_button_disabled_when_not_complete", () => {
    renderCard({ status: statusDto("in_progress") });
    expect(screen.getByRole("button", { name: "知识库更新" })).toBeDisabled();
    // 说明是一行可见文案（非 tooltip），指向需先完成知识库初始化。
    const hint = screen.getByTestId("knowledge-base-update-hint");
    expect(hint.textContent).toContain("需先完成知识库初始化");
  });

  it("update_button_disabled_when_status_missing", () => {
    // status 为 null（尚未初始化）同样视为未完成。
    renderCard({ status: null });
    expect(screen.getByRole("button", { name: "知识库更新" })).toBeDisabled();
    expect(screen.getByTestId("knowledge-base-update-hint")).toBeInTheDocument();
  });

  it("update_button_enabled_when_complete", () => {
    renderCard({ status: statusDto("complete") });
    expect(screen.getByRole("button", { name: "知识库更新" })).toBeEnabled();
    expect(
      screen.queryByTestId("knowledge-base-update-hint"),
    ).not.toBeInTheDocument();
  });

  it("update_button_disabled_when_global_validation_not_passed", () => {
    // initialization_status=complete 但 global_validation 未通过 ⇒ 与后端 update
    // 预检口径一致，按钮保持禁用（不得让用户点了才被后端拒）。
    renderCard({ status: statusDto("complete", "failed") });
    expect(screen.getByRole("button", { name: "知识库更新" })).toBeDisabled();
    expect(screen.getByTestId("knowledge-base-update-hint")).toHaveTextContent(
      "全局校验",
    );
  });

  it("update_button_click_opens_update_dialog_when_enabled", async () => {
    const user = userEvent.setup();
    renderCard({ status: statusDto("complete") });
    await user.click(screen.getByRole("button", { name: "知识库更新" }));
    expect(
      await screen.findByTestId("knowledge-base-update-dialog"),
    ).toBeInTheDocument();
  });

  it("precheck_failure_shows_missing_files", () => {
    renderCard({
      preflightFailure: {
        code: "knowledge_base_inputs_incomplete",
        message: "bootstrap inputs incomplete",
        details: { missing: ["api-scope.md"], violations: [] },
      },
    });
    const failure = screen.getByTestId("knowledge-base-preflight-failure");
    // 缺失文件名 + 稳定码对应的中文失败文案都要可见。
    expect(failure.textContent).toContain("api-scope.md");
    expect(failure.textContent).toContain("输入模板不完整");
  });

  it("precheck_failure_shows_violation_details", () => {
    renderCard({
      preflightFailure: {
        code: "knowledge_base_change_package_invalid",
        message: "change package invalid",
        details: {
          missing: [],
          violations: [
            {
              code: "field_empty",
              document: "changes/add-login.md",
              field: "scope",
              detail: "scope 字段未填写",
            },
          ],
        },
      },
    });
    const failure = screen.getByTestId("knowledge-base-preflight-failure");
    expect(failure.textContent).toContain("changes/add-login.md");
    expect(failure.textContent).toContain("scope 字段未填写");
  });

  it("create_bootstrap_inputs_button_appears_on_missing_inputs", async () => {
    const user = userEvent.setup();
    const onCreateBootstrapInputs = vi.fn();
    renderCard({
      preflightFailure: {
        code: "knowledge_base_inputs_incomplete",
        message: "bootstrap inputs incomplete",
        details: { missing: ["api-scope.md"], violations: [] },
      },
      onCreateBootstrapInputs,
    });
    const button = screen.getByRole("button", { name: "创建输入模板" });
    await user.click(button);
    expect(onCreateBootstrapInputs).toHaveBeenCalledTimes(1);
  });

  it("create_bootstrap_inputs_button_hidden_without_missing_inputs", () => {
    renderCard({ preflightFailure: null });
    expect(
      screen.queryByRole("button", { name: "创建输入模板" }),
    ).not.toBeInTheDocument();
  });

  it("dismiss_preflight_failure_calls_handler", async () => {
    const user = userEvent.setup();
    const onDismissPreflightFailure = vi.fn();
    renderCard({
      preflightFailure: {
        code: "knowledge_base_inputs_incomplete",
        message: "bootstrap inputs incomplete",
        details: { missing: ["api-scope.md"], violations: [] },
      },
      onDismissPreflightFailure,
    });
    await user.click(screen.getByRole("button", { name: "知道了" }));
    expect(onDismissPreflightFailure).toHaveBeenCalledTimes(1);
  });

  it("confirm_dialog_required_before_start", () => {
    const onStart = vi.fn();
    confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderCard({ onStart });

    fireEvent.click(screen.getByRole("button", { name: "知识库初始化" }));
    // 未确认 → 不触发。
    expect(onStart).not.toHaveBeenCalled();

    // 确认文案必须写明三件事：真实调用 provider、消耗额度、写入聚合根目录。
    const confirmText = String(confirmSpy.mock.calls[0]?.[0] ?? "");
    expect(confirmText).toContain("真实调用 provider");
    expect(confirmText).toContain("消耗额度");
    expect(confirmText).toContain("写入聚合根目录");

    confirmSpy.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "知识库初始化" }));
    expect(onStart).toHaveBeenCalledWith("bootstrap");
  });

  it("provider_summary_renders_candidates", () => {
    renderCard({ operation: operationWithProviderSelection });
    const summary = screen.getByTestId("knowledge-base-provider-selection");
    // 与聚合卡同一文案口径：「本次使用 X；跳过：Y（原因）、…」。
    expect(summary.textContent).toContain("本次使用 Pi");
    expect(summary.textContent).toContain(
      "跳过：Kimi（未核验）、Claude Code（未安装）",
    );
  });

  it("open_detail_button_calls_handler", async () => {
    const user = userEvent.setup();
    const onOpenDetail = vi.fn();
    renderCard({ onOpenDetail });
    await user.click(screen.getByRole("button", { name: "查看知识库详细内容" }));
    expect(onOpenDetail).toHaveBeenCalledTimes(1);
  });
});
