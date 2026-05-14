import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { GateActionBar } from "./GateActionBar";

describe("GateActionBar", () => {
  it("renders gate state and dispatches each gate action", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onRequestChange = vi.fn();
    const onTerminate = vi.fn();

    render(
      <GateActionBar
        gate={{ gate_id: "gate_0001", node_id: "N16", status: "blocked" }}
        onConfirm={onConfirm}
        onRequestChange={onRequestChange}
        onTerminate={onTerminate}
      />,
    );

    expect(screen.getByText("N16")).toBeInTheDocument();
    expect(screen.getByText("blocked")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "确认继续" }));
    await user.click(screen.getByRole("button", { name: "要求修改" }));
    await user.click(screen.getByRole("button", { name: "终止" }));

    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onRequestChange).toHaveBeenCalledTimes(1);
    expect(onTerminate).toHaveBeenCalledTimes(1);
  });
});
