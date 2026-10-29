import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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
  it("renders the start action and disables it with a spinner while busy", () => {
    const onStart = vi.fn();
    const view = render(
      <AggregateInitializationCard
        operation={null}
        busy={false}
        onStart={onStart}
        onCancel={vi.fn()}
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
      />,
    );

    expect(screen.getByRole("button", { name: "取消初始化" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-initialization-spinner")).toBeInTheDocument();
  });

  it("renders the completed status without a cancel or start action", () => {
    render(
      <AggregateInitializationCard
        operation={operation("completed")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
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
  });

  it("renders the failed status with its error and keeps start available for retry", () => {
    render(
      <AggregateInitializationCard
        operation={operation("failed")}
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByTestId("aggregate-initialization-status")).toHaveAttribute(
      "data-status",
      "failed",
    );
    expect(screen.getByRole("alert")).toHaveTextContent("初始化命令执行失败");
    expect(screen.getByRole("button", { name: "启动聚合初始化" })).toBeEnabled();
  });

  it("renders the cancelled status with its cancellation detail", () => {
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
      />,
    );

    expect(screen.getByTestId("aggregate-initialization-status")).toHaveAttribute(
      "data-status",
      "cancelled",
    );
    expect(screen.getByText(/operator requested stop/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动聚合初始化" })).toBeEnabled();
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
});
