import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { OperationAuditRow } from "../../state/operation-audit-projection";
import { OperationAuditView } from "./OperationAuditView";

function row(overrides: Partial<OperationAuditRow> = {}): OperationAuditRow {
  return {
    id: "audit_1",
    atMs: Date.UTC(2026, 8, 15, 12),
    atIso: "2026-09-15T12:00:00.000Z",
    operator: "本地浏览器",
    sessionId: "s1",
    gateId: "g1",
    operation: "confirm",
    source: "chat",
    outcome: "sent",
    detail: null,
    evidence: "local_command",
    ...overrides,
  };
}

describe("OperationAuditView", () => {
  it("renders operator, client time, target, operation and evidence source", () => {
    render(
      <OperationAuditView
        rows={[row()]}
        target={null}
        onTargetChange={vi.fn()}
      />,
    );

    const view = screen.getByTestId("operation-audit-view");
    expect(view).toHaveTextContent("本地浏览器");
    expect(view).toHaveTextContent("2026-09-15T12:00:00.000Z");
    expect(view).toHaveTextContent("s1 · g1");
    expect(view).toHaveTextContent("confirm");
    expect(view).toHaveTextContent("sent");
    expect(view).toHaveTextContent("本地命令日志");
  });

  it("filters rows and scopes a visible row to its target without inventing an identity", async () => {
    const onTargetChange = vi.fn();
    render(
      <OperationAuditView
        rows={[row(), row({ id: "audit_2", sessionId: "s2", gateId: "g2" })]}
        target={{ sessionId: "s1", gateId: "g1" }}
        onTargetChange={onTargetChange}
      />,
    );

    const view = screen.getByTestId("operation-audit-view");
    expect(view).toHaveTextContent("s1 · g1");
    expect(view).not.toHaveTextContent("s2 · g2");
    await userEvent.click(within(view).getByRole("button", { name: "仅此目标" }));
    expect(onTargetChange).toHaveBeenCalledWith({ sessionId: "s1", gateId: "g1" });
    expect(view).not.toHaveTextContent("未提供身份");
  });

  it("submits a session-only target and clears it", async () => {
    const onTargetChange = vi.fn();
    render(
      <OperationAuditView
        rows={[row()]}
        target={null}
        onTargetChange={onTargetChange}
      />,
    );

    await userEvent.type(screen.getByLabelText("审计会话"), "s1");
    await userEvent.click(screen.getByRole("button", { name: "按目标回查" }));
    expect(onTargetChange).toHaveBeenCalledWith({ sessionId: "s1", gateId: null });

    await userEvent.click(screen.getByRole("button", { name: "清除筛选" }));
    expect(onTargetChange).toHaveBeenLastCalledWith(null);
  });
});
