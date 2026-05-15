import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ExecutionSummaryStrip } from "./ExecutionSummaryStrip";

describe("ExecutionSummaryStrip", () => {
  it("renders compact execution metrics without hero copy", () => {
    render(
      <ExecutionSummaryStrip
        activeTaskId="task_0001"
        selectedNodeId="N16"
        nodeCount={3}
        artifactCount={2}
        eventCount={5}
      />,
    );

    expect(screen.getByRole("region", { name: "执行摘要" })).toHaveTextContent("task_0001");
    expect(screen.getByText("Nodes")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("Artifacts")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    expect(screen.queryByText("AI Coding Workbench")).not.toBeInTheDocument();
  });
});
