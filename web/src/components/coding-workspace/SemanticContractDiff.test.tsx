import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { SemanticContractDiff } from "./SemanticContractDiff";
import { planAmendmentFixture, repairAwaitingConfirmationFixture } from "./plan-repair-test-fixtures";

describe("SemanticContractDiff", () => {
  it("renders contract capability changes semantically instead of as a raw text diff", () => {
    render(
      <SemanticContractDiff
        amendment={planAmendmentFixture()}
        projection={repairAwaitingConfirmationFixture().projection}
        impact={repairAwaitingConfirmationFixture().impact}
        view="contract"
      />,
    );

    expect(screen.getByText("新增 failure_message")).toBeInTheDocument();
    expect(screen.getByText("变更 domain_error")).toBeInTheDocument();
    expect(screen.getByText("domain_result → failure_message")).toBeInTheDocument();
    expect(screen.queryByTestId("monaco-diff-viewer")).not.toBeInTheDocument();
  });

  it("shows affected coder work items in execution order", () => {
    const state = repairAwaitingConfirmationFixture();
    const projection = {
      ...state.projection!,
      coder_group_context: {
        ...state.projection!.coder_group_context,
        ordered_logical_work_item_ids: ["WI-01", "WI-02", "WI-03"],
      },
    };
    render(
      <SemanticContractDiff
        amendment={state.amendment}
        projection={projection}
        impact={state.impact}
        view="coder"
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Coder Projection 影响" }),
    ).toBeInTheDocument();
    const items = screen.getAllByTestId("semantic-coder-work-item");
    expect(items.map((item) => item.getAttribute("data-logical-id"))).toEqual([
      "WI-01",
      "WI-02",
    ]);
    expect(screen.getByText("src/domain/**")).toBeInTheDocument();
  });

  it("shows every reviewer-affected work item including revalidation-only units", () => {
    const state = repairAwaitingConfirmationFixture();
    render(
      <SemanticContractDiff
        amendment={state.amendment}
        projection={state.projection}
        impact={state.impact}
        view="reviewer"
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Reviewer Projection 影响" }),
    ).toBeInTheDocument();
    expect(screen.getByText("WI-02")).toBeInTheDocument();
    expect(screen.getByText("AC-API-ERROR")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "WI-03" })).not.toBeInTheDocument();
  });
});
