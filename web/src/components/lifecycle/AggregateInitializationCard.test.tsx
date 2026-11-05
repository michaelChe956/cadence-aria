import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AggregateInitializationOperationSnapshot } from "../../api/types";
import { AggregateInitializationCard } from "./AggregateInitializationCard";

function operation(
  status: AggregateInitializationOperationSnapshot["status"],
  overrides: Partial<AggregateInitializationOperationSnapshot> = {},
): AggregateInitializationOperationSnapshot {
  const completed = status === "completed";
  const failed = status === "failed";
  return {
    operation_id: "aggregate_initialization_0001",
    project_id: "project_0001",
    status,
    profile: null,
    steps: [
      { step_id: "machine_skills", status: completed ? "completed" : failed ? "completed" : "pending" },
      { step_id: "aggregate_preflight", status: completed ? "completed" : failed ? "failed" : "pending" },
      { step_id: "pre_check", status: completed ? "completed" : "pending" },
      { step_id: "rule_and_mcp_config", status: completed ? "completed" : "pending" },
      { step_id: "openspec_and_examples", status: completed ? "completed" : "pending" },
    ],
    current_step: completed ? null : failed ? "aggregate_preflight" : "machine_skills",
    failed_step: failed ? "aggregate_preflight" : null,
    member_projections: [],
    cancellation: null,
    error:
      status === "failed"
        ? { code: "aggregate_initialization_failed", message: "初始化命令执行失败", details: {} }
        : null,
    created_at: "2026-08-18T00:00:00Z",
    updated_at: "2026-08-18T00:00:01Z",
    completed_at: completed ? "2026-08-18T00:01:00Z" : null,
    ...overrides,
  };
}

describe("AggregateInitializationCard", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the start action and disables it with a spinner while busy", () => {
    const onStart = vi.fn();
    const view = render(
      <AggregateInitializationCard
        operation={null}
        busy={false}
        onStart={onStart}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    const start = screen.getByRole("button", { name: "启动聚合初始化" });
    expect(start).toBeEnabled();
    fireEvent.click(start);
    expect(onStart).toHaveBeenCalledOnce();

    view.rerender(
      <AggregateInitializationCard
        operation={null}
        busy
        onStart={onStart}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "启动聚合初始化" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-initialization-spinner")).toBeInTheDocument();
  });

  it("renders five steps and a cancel action while running", () => {
    render(
      <AggregateInitializationCard
        operation={operation("running")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(screen.getByTestId("aggregate-initialization-status")).toHaveTextContent("running");
    for (const stepId of [
      "machine_skills",
      "aggregate_preflight",
      "pre_check",
      "rule_and_mcp_config",
      "openspec_and_examples",
    ]) {
      expect(
        screen.getByTestId(`aggregate-initialization-step-${stepId}`),
      ).toBeInTheDocument();
    }
    expect(screen.getByRole("button", { name: "取消初始化" })).toBeEnabled();
  });

  it("disables the cancel action and shows a spinner while busy", () => {
    render(
      <AggregateInitializationCard
        operation={operation("running")}
        busy
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "取消初始化" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-initialization-spinner")).toBeInTheDocument();
  });

  it("renders the completed status with a reinitialize action and no cancel or start action", () => {
    render(
      <AggregateInitializationCard
        operation={operation("completed")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(screen.getByTestId("aggregate-initialization-status")).toHaveAttribute(
      "data-status",
      "completed",
    );
    expect(screen.getByText("初始化完成")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "取消初始化" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "启动聚合初始化" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "重新初始化" }),
    ).toBeEnabled();
  });

  it("renders the failed status with its error and offers the reinitialize action", () => {
    render(
      <AggregateInitializationCard
        operation={operation("failed")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(screen.getByTestId("aggregate-initialization-status")).toHaveAttribute(
      "data-status",
      "failed",
    );
    expect(screen.getByRole("alert")).toHaveTextContent("初始化命令执行失败");
    expect(
      screen.getByRole("button", { name: "重新初始化" }),
    ).toBeEnabled();
    // 四态互斥：失败态不再提供「启动聚合初始化」（重跑入口唯一）。
    expect(
      screen.queryByRole("button", { name: "启动聚合初始化" }),
    ).not.toBeInTheDocument();
  });

  it("renders the cancelled status with its cancellation detail and the reinitialize action", () => {
    render(
      <AggregateInitializationCard
        operation={operation("cancelled", {
          cancellation: {
            reason_code: "user_cancelled",
            cancelled_at: "2026-08-18T00:01:00Z",
            detail: "operator requested stop",
          },
        })}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(screen.getByTestId("aggregate-initialization-status")).toHaveAttribute(
      "data-status",
      "cancelled",
    );
    expect(screen.getByText(/operator requested stop/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "重新初始化" }),
    ).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "启动聚合初始化" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the four operation states mutually exclusive in the action bar", () => {
    // null → 只给「启动聚合初始化」；created|running → 只给「取消初始化」；
    // completed|failed|cancelled → 只给「重新初始化」。
    const cases: Array<{
      operation: AggregateInitializationOperationSnapshot | null;
      visible: string;
      absent: string[];
    }> = [
      { operation: null, visible: "启动聚合初始化", absent: ["取消初始化", "重新初始化"] },
      {
        operation: operation("created"),
        visible: "取消初始化",
        absent: ["启动聚合初始化", "重新初始化"],
      },
      {
        operation: operation("running"),
        visible: "取消初始化",
        absent: ["启动聚合初始化", "重新初始化"],
      },
      {
        operation: operation("completed"),
        visible: "重新初始化",
        absent: ["启动聚合初始化", "取消初始化"],
      },
      {
        operation: operation("failed"),
        visible: "重新初始化",
        absent: ["启动聚合初始化", "取消初始化"],
      },
      {
        operation: operation("cancelled"),
        visible: "重新初始化",
        absent: ["启动聚合初始化", "取消初始化"],
      },
    ];

    for (const item of cases) {
      const view = render(
        <AggregateInitializationCard
          operation={item.operation}
          busy={false}
          onStart={vi.fn()}
          onCancel={vi.fn()}
          onReinitialize={vi.fn()}
        />,
      );
      expect(
        screen.getByRole("button", { name: item.visible }),
      ).toBeInTheDocument();
      for (const name of item.absent) {
        expect(screen.queryByRole("button", { name })).not.toBeInTheDocument();
      }
      view.unmount();
    }
  });

  it("asks for confirmation with the full-recipe wording before reinitializing", () => {
    const onReinitialize = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(
      <AggregateInitializationCard
        operation={operation("completed")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={onReinitialize}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "重新初始化" }));

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    const confirmText = String(confirmSpy.mock.calls[0]?.[0] ?? "");
    // 二次确认必须写明：强制重跑完整五步 / 四条 Cadence-skills 命令 /
    // 适用于 skills 更新后重装 / 会发布新的聚合政策版本。
    expect(confirmText).toMatch(/完整五步/);
    expect(confirmText).toMatch(/Cadence-skills/);
    expect(confirmText).toMatch(/更新后重装/);
    expect(confirmText).toMatch(/版本/);
    for (const command of [
      "/pre-check",
      "/rule-config",
      "/mcp-configuration",
      "/project-rules-examples",
    ]) {
      expect(confirmText).toContain(command);
    }
    // 取消确认 → 不触发重跑。
    expect(onReinitialize).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "重新初始化" }));
    expect(onReinitialize).toHaveBeenCalledTimes(1);
  });

  it("disables the reinitialize action while busy", () => {
    render(
      <AggregateInitializationCard
        operation={operation("completed")}
        busy
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "重新初始化" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-initialization-spinner")).toBeInTheDocument();
  });

  it("renders skipped steps as skipped with a label instead of pending or running", () => {
    // 聚合根已完成 Cadence-skills 初始化 → 后端把三个 provider-turn 步骤
    // 标为 skipped（见后端 skip_skills 测试的 ["completed","completed","skipped",...]）。
    render(
      <AggregateInitializationCard
        operation={operation("completed", {
          steps: [
            { step_id: "machine_skills", status: "completed" },
            { step_id: "aggregate_preflight", status: "completed" },
            { step_id: "pre_check", status: "skipped" },
            { step_id: "rule_and_mcp_config", status: "skipped" },
            { step_id: "openspec_and_examples", status: "skipped" },
          ],
        })}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    for (const stepId of [
      "pre_check",
      "rule_and_mcp_config",
      "openspec_and_examples",
    ]) {
      const step = screen.getByTestId(`aggregate-initialization-step-${stepId}`);
      expect(step).toHaveAttribute("data-status", "skipped");
      expect(
        step.querySelector(
          '[data-testid="aggregate-initialization-step-indicator"]',
        ),
      ).toHaveAttribute("data-status", "skipped");
      expect(within(step).getByText("已跳过")).toBeInTheDocument();
    }

    // 「已跳过」不得被渲染成 running / completed 样式：指示器自身不带
    // spinner（running）或成功色（completed）。
    const skippedIndicator = screen
      .getByTestId("aggregate-initialization-step-pre_check")
      .querySelector('[data-testid="aggregate-initialization-step-indicator"]');
    expect(skippedIndicator?.querySelector(".animate-spin")).toBeNull();
    expect(skippedIndicator?.innerHTML).not.toContain("--aria-success");
    expect(skippedIndicator?.innerHTML).not.toContain("--aria-primary");
    expect(skippedIndicator?.innerHTML).toContain("--aria-ink-muted");

    // 跳过的步骤带解释文案（tooltip），用户能看懂「为什么这三步没跑」。
    const hints = screen.getAllByTestId(
      "aggregate-initialization-step-skipped-label",
    );
    expect(hints).toHaveLength(3);
    for (const hint of hints) {
      expect(hint.getAttribute("title")).toContain("跳过");
    }
  });

  it("renders the frozen provider selection summary when provider_selection exists", () => {
    render(
      <AggregateInitializationCard
        operation={operation("completed", {
          provider_selection: {
            selected: "pi",
            candidates: [
              { provider: "pi", display_name: "Pi", cli_available: true, cli_reason_code: null, capability_state: "confirmed", skipped_reason: null, action_hint: "直接使用" },
              { provider: "kimi_code", display_name: "Kimi", cli_available: true, cli_reason_code: null, capability_state: "unverified", skipped_reason: "needs_verification", action_hint: "核验 Kimi" },
              { provider: "claude_code", display_name: "Claude Code", cli_available: false, cli_reason_code: "command_missing", capability_state: "not_installed", skipped_reason: "cli_unavailable", action_hint: "安装 Claude Code" },
              { provider: "codex", display_name: "Codex", cli_available: true, cli_reason_code: null, capability_state: "confirmed", skipped_reason: null, action_hint: "直接使用" },
            ],
            rule_version: "v1",
            decided_at: "2026-11-03T00:00:00Z",
          },
        })}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    const summary = screen.getByTestId("aggregate-initialization-provider-selection");
    expect(summary.textContent).toContain("本次使用 Pi");
    expect(summary.textContent).toContain("跳过：Kimi（未核验）、Claude Code（未安装）");
    // 未被跳过的候选（codex）不进入跳过清单。
    expect(summary.textContent).not.toContain("Codex");
  });

  it("does not render the provider selection summary when provider_selection is null", () => {
    render(
      <AggregateInitializationCard
        operation={operation("completed", { provider_selection: null })}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );
    expect(
      screen.queryByTestId("aggregate-initialization-provider-selection"),
    ).not.toBeInTheDocument();
  });

  // 停止原因可见化（owner 2026-11-04）：点「启动聚合初始化」失败时，卡片上
  // 必须能看到「卡在哪一步、为什么、属于哪道闸门」——不是一个无解释的按钮。
  it("renders the advance stop reason next to the start button", () => {
    render(
      <AggregateInitializationCard
        operation={null}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
        advanceStop={{
          step: "rules_policy",
          reason: "waiting_for_human",
          reason_code: "aggregate_initialization_provider_needs_verification",
          detail: "provider 能力未核验",
          stopped_at: "2026-11-04T00:00:00Z",
        }}
      />,
    );

    const stop = screen.getByTestId("aggregate-initialization-advance-stop");
    const headline = screen.getByTestId(
      "aggregate-initialization-advance-stop-headline",
    );
    // 停在哪一步（中文口径，与冷启动卡片同一套标签）。
    expect(headline.textContent).toContain("规则/政策");
    // 为什么（停止枚举的中文口径）。
    expect(headline.textContent).toContain("等待人工处理");
    // 属于哪道闸门（稳定机器码 → 短标签）。
    expect(
      screen.getByTestId("aggregate-initialization-advance-stop-gate").textContent,
    ).toContain("需先核验 provider");
    // 机器码本身不重复渲染（由 bootstrap 通知卡承担）。
    expect(stop.textContent).not.toContain(
      "aggregate_initialization_provider_needs_verification",
    );
  });

  it("does not render the advance stop block when advanceStop is absent", () => {
    render(
      <AggregateInitializationCard
        operation={null}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );
    expect(
      screen.queryByTestId("aggregate-initialization-advance-stop"),
    ).not.toBeInTheDocument();
  });
  it("renders the aggregate initialization provider order notice without Kimi Code", () => {
    render(
      <AggregateInitializationCard
        operation={operation("running")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onReinitialize={vi.fn()}
      />,
    );

    expect(
      screen.getByTestId("aggregate-initialization-aggregate-order-notice"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("aggregate-initialization-aggregate-order-notice")
        .textContent,
    ).toBe(
      "聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。",
    );
  });
});
