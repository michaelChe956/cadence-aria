import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ReviewDecisionStagePanel } from "./ReviewDecisionStagePanel";

describe("ReviewDecisionStagePanel", () => {
  it("shows three options", () => {
    render(
      <ReviewDecisionStagePanel
        reviewer="codex"
        verdict="revise"
        summary="缺少边界场景"
        onSelectPath={vi.fn()}
      />,
    );

    expect(screen.getByTestId("review-decision-panel")).toBeInTheDocument();
    expect(screen.getByText("直接返修")).toBeInTheDocument();
    expect(screen.getByText("补充上下文后返修")).toBeInTheDocument();
    expect(screen.getByText("跳过审核结论，进入人工确认")).toBeInTheDocument();
  });

  it("calls onSelectPath with revise by default", () => {
    const onSelect = vi.fn();
    render(
      <ReviewDecisionStagePanel
        reviewer="codex"
        verdict="revise"
        summary="缺少边界场景"
        onSelectPath={onSelect}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "确定路径" }));

    expect(onSelect).toHaveBeenCalledWith("revise", undefined);
  });

  it("sends extra context for revise-with-context", () => {
    const onSelect = vi.fn();
    render(
      <ReviewDecisionStagePanel
        reviewer="codex"
        verdict="revise"
        summary="缺少边界场景"
        onSelectPath={onSelect}
      />,
    );

    fireEvent.click(screen.getByLabelText("补充上下文后返修"));
    fireEvent.change(screen.getByLabelText("补充上下文"), {
      target: { value: "补充移动端边界条件" },
    });
    fireEvent.click(screen.getByRole("button", { name: "确定路径" }));

    expect(onSelect).toHaveBeenCalledWith("revise-with-context", "补充移动端边界条件");
  });
});
