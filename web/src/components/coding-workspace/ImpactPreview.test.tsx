import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ImpactPreview } from "./ImpactPreview";
import { repairAwaitingConfirmationFixture } from "./plan-repair-test-fixtures";

describe("ImpactPreview", () => {
  it("shows execution, revalidation, unaffected, and dependency path impact", () => {
    const state = repairAwaitingConfirmationFixture();
    render(
      <ImpactPreview
        amendment={state.amendment}
        impact={state.impact}
        projection={state.projection}
      />,
    );

    expect(screen.getByText("WI-01：重新执行")).toBeInTheDocument();
    expect(screen.getByText("WI-02：重新验证")).toBeInTheDocument();
    expect(screen.queryByText("WI-03：不受影响")).not.toBeInTheDocument();
    expect(screen.getByText("WI-03：条件性下游")).toBeInTheDocument();
    expect(screen.getByText("WI-01 → WI-02 · domain_result")).toBeInTheDocument();
  });
});
