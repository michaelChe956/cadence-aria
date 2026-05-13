import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EvidencePanel } from "./EvidencePanel";

describe("EvidencePanel", () => {
  it("shows clear empty states for artifacts and diagnostics", () => {
    render(<EvidencePanel artifacts={[]} diagnostics={[]} />);

    expect(screen.getByText("暂无产物")).toBeInTheDocument();
    expect(screen.getByText("暂无诊断")).toBeInTheDocument();
  });

  it("renders artifacts as coding evidence cards with a visual preview", () => {
    render(
      <EvidencePanel
        artifacts={[
          {
            artifact_ref: "report",
            artifact_kind: "markdown",
            path: "cadence/report.md",
          },
        ]}
        diagnostics={[]}
      />,
    );

    expect(screen.getByRole("img", { name: "artifact preview" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Evidence" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /markdown/ })).toHaveTextContent(
      "cadence/report.md",
    );
    expect(screen.queryByText(/学习/)).not.toBeInTheDocument();
  });
});
