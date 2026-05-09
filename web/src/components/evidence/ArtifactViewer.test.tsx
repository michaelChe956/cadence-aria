import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ArtifactViewer } from "./ArtifactViewer";

describe("ArtifactViewer", () => {
  it("renders json artifact content with path and producer node", () => {
    render(
      <ArtifactViewer
        artifact={{
          artifact_ref: "coding_report_work_wt_001_0001",
          artifact_kind: "coding_report",
          producer_node: "N16",
          path: ".aria/runtime/tasks/task_0001/artifacts/execution/0000.json",
          content_type: "json",
          content: "{\"status\":\"completed\"}",
        }}
      />,
    );
    expect(screen.getByText("coding_report")).toBeInTheDocument();
    expect(screen.getByText(/N16/)).toBeInTheDocument();
    expect(screen.getByText(/completed/)).toBeInTheDocument();
  });
});
