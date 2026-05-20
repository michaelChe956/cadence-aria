import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { HumanConfirmStagePanel } from "./HumanConfirmStagePanel";

describe("HumanConfirmStagePanel", () => {
  it("shows reviewer summary and line diff", () => {
    render(
      <HumanConfirmStagePanel
        artifactVersion={{ version: 2, markdown: "# v2\n新增内容" }}
        reviewerSummary={{ verdict: "pass", points: ["边界场景已补齐"] }}
        prevVersion={{ version: 1, markdown: "# v1" }}
        onConfirm={vi.fn()}
        onRequestChange={vi.fn()}
        onTerminate={vi.fn()}
      />,
    );

    expect(screen.getByTestId("human-confirm-panel")).toBeInTheDocument();
    expect(screen.getByText("边界场景已补齐")).toBeInTheDocument();
    expect(screen.getByText(/v1 → v2/)).toBeInTheDocument();
    expect(screen.getByText(/新增 2 行/)).toBeInTheDocument();
  });

  it("submits structured feedback on request change", () => {
    const onRequestChange = vi.fn();
    render(
      <HumanConfirmStagePanel
        artifactVersion={{ version: 2, markdown: "# v2" }}
        reviewerSummary={{ verdict: "pass", points: [] }}
        onConfirm={vi.fn()}
        onRequestChange={onRequestChange}
        onTerminate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "要求修改" }));
    fireEvent.click(screen.getByLabelText("内容缺失"));
    fireEvent.change(screen.getByLabelText("具体描述"), {
      target: { value: "缺少错误处理" },
    });
    fireEvent.click(screen.getByRole("button", { name: "提交" }));

    expect(onRequestChange).toHaveBeenCalledWith({
      feedback_types: ["内容缺失"],
      description: "缺少错误处理",
      target_artifact_version: 2,
    });
  });

  it("calls confirm and terminate actions", () => {
    const onConfirm = vi.fn();
    const onTerminate = vi.fn();

    render(
      <HumanConfirmStagePanel
        artifactVersion={{ version: 2, markdown: "# v2" }}
        reviewerSummary={{ verdict: "pass", points: [] }}
        onConfirm={onConfirm}
        onRequestChange={vi.fn()}
        onTerminate={onTerminate}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));

    expect(onConfirm).toHaveBeenCalled();
    expect(onTerminate).toHaveBeenCalled();
  });
});
