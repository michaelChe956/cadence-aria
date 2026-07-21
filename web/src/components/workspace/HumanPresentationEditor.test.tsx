import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { HumanPresentationEditor } from "./HumanPresentationEditor";

describe("HumanPresentationEditor", () => {
  it("saves an informative explanation without exposing normative controls", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    render(<HumanPresentationEditor base={humanProjectionFixture()} onSave={onSave} />);

    await user.clear(screen.getByLabelText("拆分说明"));
    await user.type(
      screen.getByLabelText("拆分说明"),
      "先稳定核心状态机，再接 API",
    );
    await user.click(screen.getByRole("button", { name: "保存说明" }));

    expect(onSave).toHaveBeenCalledWith({
      type: "save_human_presentation_revision",
      source_projection_bundle_id: "plan-projection-001",
      scope: "plan",
      supersedes: null,
      human_summary: "先稳定核心状态机，再接 API",
      why_split: "按契约边拆分",
      dependency_explanation: ["WI-01 → WI-02"],
      risk_explanation: ["并发写入风险"],
      source_refs: ["story:repository-init"],
    });
    expect(screen.queryByLabelText("修改 Coder Contract")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Normative")).not.toBeInTheDocument();
  });

  it("supports keyboard save and announces busy and recoverable errors", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    const { rerender } = render(
      <HumanPresentationEditor base={humanProjectionFixture()} onSave={onSave} />,
    );

    screen.getByLabelText("拆分说明").focus();
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(onSave).toHaveBeenCalledTimes(1);

    rerender(
      <HumanPresentationEditor
        base={humanProjectionFixture()}
        onSave={onSave}
        saving
      />,
    );
    expect(screen.getByRole("button", { name: "保存中…" })).toBeDisabled();
    expect(screen.getByRole("form", { name: "编辑人工说明" })).toHaveAttribute(
      "aria-busy",
      "true",
    );

    rerender(
      <HumanPresentationEditor
        base={humanProjectionFixture()}
        onSave={onSave}
        error="说明已被其他编辑覆盖，请基于最新版本重试"
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "说明已被其他编辑覆盖，请基于最新版本重试",
    );
  });
});

function humanProjectionFixture() {
  return {
    scope: "plan" as const,
    source_projection_bundle_id: "plan-projection-001",
    human_summary: "原始拆分说明",
    why_split: "按契约边拆分",
    dependency_explanation: ["WI-01 → WI-02"],
    risk_explanation: ["并发写入风险"],
    source_refs: ["story:repository-init"],
    presentation: null,
  };
}
