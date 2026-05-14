import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AutoActionStatus } from "./AutoActionStatus";

describe("AutoActionStatus", () => {
  it("shows current auto action event summary and stop control", async () => {
    const onStop = vi.fn();
    render(
      <AutoActionStatus
        currentAction="N00 初始化任务状态"
        events={[
          { cursor: 1, event_type: "node_started", task_id: "task_0001", payload: { node_id: "N00" } },
          {
            cursor: 2,
            event_type: "artifact_written",
            task_id: "task_0001",
            payload: { artifact_ref: "intake_brief_0001" },
          },
        ]}
        onStop={onStop}
      />,
    );

    expect(screen.getByText("N00 初始化任务状态")).toBeInTheDocument();
    expect(screen.getByText(/node_started/)).toBeInTheDocument();
    expect(screen.getByText(/artifact_written/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "停止" }));
    expect(onStop).toHaveBeenCalled();
  });
});
