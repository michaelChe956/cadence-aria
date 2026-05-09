import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TaskSwitcher } from "./TaskSwitcher";

describe("TaskSwitcher", () => {
  it("renders existing tasks and selects one to continue", async () => {
    const onSelectTask = vi.fn();
    render(
      <TaskSwitcher
        tasks={[
          { task_id: "task_0001", change_id: "aria-fibonacci-square", phase: "blocked_by_gate" },
          { task_id: "task_0002", change_id: "aria-login-jwt", phase: "execution" },
        ]}
        activeTaskId="task_0001"
        onSelectTask={onSelectTask}
      />,
    );

    await userEvent.selectOptions(screen.getByLabelText("继续任务"), "task_0002");
    expect(onSelectTask).toHaveBeenCalledWith("task_0002");
  });
});
