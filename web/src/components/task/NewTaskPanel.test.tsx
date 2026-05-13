import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { NewTaskPanel } from "./NewTaskPanel";

describe("NewTaskPanel", () => {
  it("submits request change policy provider and timeout", async () => {
    const onCreateTask = vi.fn();
    render(<NewTaskPanel onCreateTask={onCreateTask} busy={false} />);

    await userEvent.type(screen.getByLabelText("任务请求"), "实现 Fibonacci square sum");
    await userEvent.type(screen.getByLabelText("change id"), "aria-fibonacci-square");
    await userEvent.selectOptions(screen.getByLabelText("policy preset"), "manual-write");
    await userEvent.selectOptions(screen.getByLabelText("provider mode"), "fake");
    await userEvent.clear(screen.getByLabelText("timeout seconds"));
    await userEvent.type(screen.getByLabelText("timeout seconds"), "2400");
    await userEvent.click(screen.getByRole("button", { name: "新建任务" }));

    expect(onCreateTask).toHaveBeenCalledWith({
      request_text: "实现 Fibonacci square sum",
      change_id: "aria-fibonacci-square",
      policy_preset: "manual-write",
      provider_mode: "fake",
      timeout_secs: 2400,
    });
  });

  it("preserves AI coding workbench terms in grouped execution settings", () => {
    render(<NewTaskPanel onCreateTask={() => undefined} busy={false} />);

    expect(screen.getByRole("heading", { name: "Workspace" })).toHaveClass("text-[#241B2F]");
    expect(screen.getByRole("group", { name: "Execution settings" })).toBeInTheDocument();
    expect(screen.getByText("任务请求")).toBeInTheDocument();
    expect(screen.getByText("change id")).toBeInTheDocument();
    expect(screen.getByText("policy preset")).toBeInTheDocument();
    expect(screen.getByText("provider mode")).toBeInTheDocument();
    expect(screen.queryByText(/学习|课程|助教/)).not.toBeInTheDocument();
  });
});
