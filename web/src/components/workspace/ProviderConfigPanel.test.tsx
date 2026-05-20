import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProviderConfigPanel } from "./ProviderConfigPanel";

describe("ProviderConfigPanel", () => {
  it("renders author and reviewer selects with reviewer enabled", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Author")).toBeInTheDocument();
    expect(screen.getByLabelText("Reviewer")).toBeInTheDocument();
    expect(screen.getByLabelText("启用交叉审核")).toBeChecked();
    expect(screen.getByText("可编辑")).toBeInTheDocument();
  });

  it("shows a quality warning when reviewer is disabled", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={false}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByText("未启用交叉审核可能降低 artifact 质量")).toBeInTheDocument();
    expect(screen.queryByLabelText("Reviewer")).not.toBeInTheDocument();
  });

  it("disables provider controls when not editable", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={false}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Author")).toBeDisabled();
    expect(screen.getByLabelText("Reviewer")).toBeDisabled();
    expect(screen.getByLabelText("启用交叉审核")).toBeDisabled();
    expect(screen.getByText("已锁定")).toBeInTheDocument();
  });

  it("emits provider, reviewer toggle, and review round changes", () => {
    const onSelectProvider = vi.fn();
    const onToggleReviewer = vi.fn();
    const onChangeRounds = vi.fn();
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        rounds={1}
        onSelectProvider={onSelectProvider}
        onToggleReviewer={onToggleReviewer}
        onChangeRounds={onChangeRounds}
      />,
    );

    fireEvent.change(screen.getByLabelText("Author"), { target: { value: "fake" } });
    fireEvent.click(screen.getByLabelText("启用交叉审核"));
    fireEvent.click(screen.getByRole("button", { name: "高级配置" }));
    fireEvent.change(screen.getByLabelText("审核轮次"), { target: { value: "2" } });

    expect(onSelectProvider).toHaveBeenCalledWith("author", "fake");
    expect(onToggleReviewer).toHaveBeenCalledWith(false);
    expect(onChangeRounds).toHaveBeenCalledWith(2);
  });
});
