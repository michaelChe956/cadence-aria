import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EvidencePanel } from "./EvidencePanel";

describe("EvidencePanel", () => {
  it("groups artifacts and diagnostics for the selected node", () => {
    render(
      <EvidencePanel
        artifacts={[
          {
            artifact_ref: "coding_report_work_wt_001_0001",
            artifact_kind: "coding_report",
            producer_node: "N16",
            path: ".aria/report.json",
            content_type: "json",
            dropped: false,
          },
        ]}
        diagnostics={[{ code: "gate_blocked", message: "archive worktask failed", node_id: "N18" }]}
      />,
    );
    expect(screen.getByText("coding_report")).toBeInTheDocument();
    expect(screen.getByText("archive worktask failed")).toBeInTheDocument();
  });
});
