import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { NodeWorkspace } from "./NodeWorkspace";

describe("NodeWorkspace", () => {
  it("surfaces selected node summary above the detailed tabs", () => {
    render(
      <NodeWorkspace
        context={{
          node_id: "N17",
          overview: {
            node_id: "N17",
            status: "running",
            provider_type: "codex",
            attempt: 2,
            artifact_count: 3,
          },
          inputs: [],
          run: [],
          outputs: [],
          diffs: [],
        }}
        selectedTab="overview"
        onSelectTab={() => undefined}
      />,
    );

    const summary = screen.getByRole("group", { name: "当前节点摘要" });
    expect(within(summary).getByText("N17")).toBeInTheDocument();
    expect(within(summary).getByText("running")).toBeInTheDocument();
    expect(within(summary).getByText("codex")).toBeInTheDocument();
    expect(within(summary).getByText("3")).toBeInTheDocument();
  });
});
