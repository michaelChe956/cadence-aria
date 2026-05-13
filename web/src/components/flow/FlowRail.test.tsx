import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { FlowRail } from "./FlowRail";

describe("FlowRail", () => {
  it("shows a compact empty state before a workflow timeline exists", () => {
    render(<FlowRail timeline={[]} selectedNodeId={null} onSelectNode={() => undefined} />);

    expect(screen.getByRole("navigation", { name: "Workflow map" })).toHaveTextContent(
      "暂无 workflow 节点",
    );
    expect(screen.queryByRole("button", { name: /N00/ })).not.toBeInTheDocument();
  });

  it("renders a clickable workflow map with node status, edges, and dropped history", async () => {
    const selected: string[] = [];
    render(
      <FlowRail
        timeline={[
          {
            node_id: "N16",
            status: "completed",
            provider_type: "codex",
            dropped: false,
            attempt: 2,
            rework_count: 1,
            artifact_count: 3,
            diagnostic: "gate_blocked",
          },
          {
            node_id: "N17",
            status: "dropped",
            provider_type: "internal",
            dropped: true,
            attempt: 1,
            rework_count: 0,
            artifact_count: 0,
          },
        ]}
        selectedNodeId="N16"
        onSelectNode={(nodeId) => selected.push(nodeId)}
      />,
    );
    expect(screen.getByRole("navigation", { name: "Workflow map" })).toBeInTheDocument();
    expect(screen.getByTestId("workflow-path-rail")).toHaveAttribute("data-motion", "ambient");
    expect(screen.getByTestId("workflow-edge-N16-N17")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /N16/ })).toHaveAttribute("data-active", "true");
    expect(screen.getByRole("button", { name: /N16/ })).toHaveTextContent("completed");
    expect(screen.getByRole("button", { name: /N16/ })).toHaveTextContent("attempt 2");
    expect(screen.getByRole("button", { name: /N16/ })).toHaveTextContent("rework 1");
    expect(screen.getByRole("button", { name: /N16/ })).toHaveTextContent("artifacts 3");
    expect(screen.getByRole("button", { name: /N16/ })).toHaveTextContent("gate_blocked");
    expect(screen.getByRole("button", { name: /N17/ })).toHaveAttribute("data-dropped", "true");

    await userEvent.click(screen.getByRole("button", { name: /N17/ }));
    expect(selected).toEqual(["N17"]);
  });
});
