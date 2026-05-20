import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { StageActionsBar } from "./StageActionsBar";

describe("StageActionsBar", () => {
  it("shows start generation button in prepare_context", () => {
    const onStart = vi.fn();

    render(<StageActionsBar stage="prepare_context" onStartGeneration={onStart} />);
    fireEvent.click(screen.getByRole("button", { name: "开始生成" }));

    expect(onStart).toHaveBeenCalled();
  });

  it("shows abort button in running", () => {
    const onAbort = vi.fn();

    render(<StageActionsBar stage="running" onAbort={onAbort} />);
    fireEvent.click(screen.getByRole("button", { name: "中止" }));

    expect(onAbort).toHaveBeenCalled();
  });

  it("shows review decision actions", () => {
    const onSelectRevisionPath = vi.fn();
    const onAbort = vi.fn();

    render(
      <StageActionsBar
        stage="review_decision"
        onSelectRevisionPath={onSelectRevisionPath}
        onAbort={onAbort}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "选择修订路径" }));
    fireEvent.click(screen.getByRole("button", { name: "中止" }));

    expect(onSelectRevisionPath).toHaveBeenCalledWith("revise");
    expect(onAbort).toHaveBeenCalled();
  });

  it("shows confirm, request change, and terminate in human_confirm", () => {
    const onConfirm = vi.fn();
    const onRequestChange = vi.fn();
    const onTerminate = vi.fn();

    render(
      <StageActionsBar
        stage="human_confirm"
        onConfirm={onConfirm}
        onRequestChange={onRequestChange}
        onTerminate={onTerminate}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    fireEvent.click(screen.getByRole("button", { name: "要求修改" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));

    expect(onConfirm).toHaveBeenCalled();
    expect(onRequestChange).toHaveBeenCalled();
    expect(onTerminate).toHaveBeenCalled();
  });
});
