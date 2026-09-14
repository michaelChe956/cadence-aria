// web/src/components/coding-workspace/dashboard/CodingBudgetGates.test.tsx
import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CodingBudgetGate } from "../../../state/coding-dashboard-projection";
import { CodingBudgetGates } from "./CodingBudgetGates";

const NOW = Date.parse("2026-09-14T08:52:00Z");

function gate(overrides: Partial<CodingBudgetGate> = {}): CodingBudgetGate {
  return {
    kind: "work_item",
    label: "Work Item 预算门（60min）",
    totalMs: 3_600_000,
    anchorAtMs: Date.parse("2026-09-14T08:00:00Z"),
    endMs: NOW,
    elapsedMs: 3_120_000,
    remainingMs: 480_000,
    ratio: 3_120_000 / 3_600_000,
    nearExhaustion: true,
    frozen: false,
    ...overrides,
  };
}

describe("CodingBudgetGates", () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => vi.useRealTimers());

  it("renders progress, remaining time and the frontend-timing disclaimer", () => {
    render(<CodingBudgetGates gates={[gate({ nearExhaustion: false })]} nowMs={NOW} />);
    expect(screen.getByText("Work Item 预算门（60min）")).toBeInTheDocument();
    expect(screen.getByTestId("coding-budget-bar-work_item")).toHaveStyle({ width: "87%" });
    expect(screen.getByTestId("coding-budget-remaining-work_item")).toHaveTextContent("剩 8m0s");
    const disclaimer = screen.getByTestId("coding-budget-disclaimer").textContent ?? "";
    expect(disclaimer).toContain("前端自计时");
    expect(disclaimer).toContain("非引擎事件数据");
  });

  it("turns the bar orange and announces through an alert when near exhaustion", () => {
    render(<CodingBudgetGates gates={[gate()]} nowMs={NOW} />);
    expect(screen.getByTestId("coding-budget-bar-work_item")).toHaveStyle({ background: "var(--aria-warning)" });
    expect(screen.getByRole("alert")).toHaveTextContent("剩余不足 10 分钟");
  });

  it("keeps the primary colour and stays silent when no gate is near exhaustion", () => {
    render(<CodingBudgetGates gates={[gate({ nearExhaustion: false })]} nowMs={NOW} />);
    expect(screen.getByTestId("coding-budget-bar-work_item")).toHaveStyle({ background: "var(--aria-primary)" });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("repeats the announcement at the configured escalation interval", () => {
    render(<CodingBudgetGates gates={[gate()]} nowMs={NOW} />);
    expect(screen.getByRole("alert")).toHaveTextContent("第 1 次提醒");
    act(() => vi.advanceTimersByTime(300_000 + 10));
    expect(screen.getByRole("alert")).toHaveTextContent("第 2 次提醒");
  });

  it("shows elapsed-total copy when a gate is fully drained", () => {
    render(
      <CodingBudgetGates
        gates={[gate({ remainingMs: 0, elapsedMs: 3_600_000, ratio: 1, nearExhaustion: true })]}
        nowMs={NOW}
      />,
    );
    expect(screen.getByTestId("coding-budget-remaining-work_item")).toHaveTextContent("已耗尽");
  });

  it("withdraws the announcement once the budget freezes at completion", () => {
    render(<CodingBudgetGates gates={[gate({ frozen: true })]} nowMs={NOW} />);
    expect(screen.queryByRole("alert")).toBeNull();
    act(() => vi.advanceTimersByTime(300_000 + 10));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps announcing while a near gate is still running before completion", () => {
    render(<CodingBudgetGates gates={[gate({ frozen: false })]} nowMs={NOW} />);
    expect(screen.getByRole("alert")).toHaveTextContent("第 1 次提醒");
    expect(screen.getByRole("alert")).toHaveTextContent("剩余不足 10 分钟");
  });

  it("renders an explicit empty state before the coding stage begins", () => {
    render(<CodingBudgetGates gates={[]} nowMs={NOW} />);
    expect(screen.getByTestId("coding-budget-empty")).toHaveTextContent("尚未进入 Coding 阶段");
  });
});
