// web/src/components/coding-workspace/dashboard/CodingTopologyGraph.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CodingTopology } from "../../../state/coding-dashboard-projection";
import { CodingTopologyGraph } from "./CodingTopologyGraph";

const topology: CodingTopology = {
  nodes: [
    { workItemId: "wi_a", unitId: "u1", title: "单元A", orderIndex: 0, state: "done", completionCommit: "abc123" },
    { workItemId: "wi_b", unitId: "u2", title: "单元B", orderIndex: 1, state: "running", completionCommit: null },
    { workItemId: "wi_c", unitId: "u3", title: "单元C", orderIndex: 2, state: "awaiting_triage", completionCommit: null },
  ],
  edges: [
    { fromWorkItemId: "wi_a", toWorkItemId: "wi_b", state: "satisfied" },
    { fromWorkItemId: "wi_b", toWorkItemId: "wi_c", state: "blocking" },
  ],
};

function renderGraph(overrides: Partial<Parameters<typeof CodingTopologyGraph>[0]> = {}) {
  return render(
    <CodingTopologyGraph
      topology={topology}
      selectedWorkItemId={null}
      onSelectWorkItem={() => undefined}
      {...overrides}
    />,
  );
}

describe("CodingTopologyGraph", () => {
  it("renders six-state nodes with topology tokens and edges labelled by state", () => {
    const { container } = renderGraph();
    const graph = screen.getByTestId("coding-topology-graph");
    expect(graph.querySelector('[data-node="wi_a"]')).toHaveAttribute("data-state", "done");
    expect(graph.querySelector('[data-node="wi_b"]')).toHaveAttribute("data-state", "running");
    expect(graph.querySelector('[data-node="wi_c"]')).toHaveAttribute("data-state", "awaiting_triage");
    expect(graph.querySelector('[data-node="wi_a"] rect')).toHaveAttribute("fill", "var(--aria-topo-node-done-bg)");
    expect(graph.querySelector('[data-edge="wi_a__wi_b"]')).toHaveAttribute("data-edge-state", "satisfied");
    expect(graph.querySelector('[data-edge="wi_b__wi_c"]')).toHaveAttribute("data-edge-state", "blocking");
    expect(graph.querySelector('[data-edge="wi_b__wi_c"]')).toHaveAttribute("stroke", "var(--aria-topo-node-blocked-fg)");
    expect(container.querySelector("svg")).toBeInTheDocument();
  });

  it("marks the selected node with the active edge token", () => {
    renderGraph({ selectedWorkItemId: "wi_b" });
    expect(screen.getByTestId("coding-topology-graph").querySelector('[data-node="wi_b"] rect')).toHaveAttribute(
      "stroke",
      "var(--aria-topo-edge-active)",
    );
  });

  it("selects a node on click and via keyboard", async () => {
    const user = userEvent.setup();
    const onSelectWorkItem = vi.fn();
    renderGraph({ onSelectWorkItem });
    await user.click(screen.getByRole("button", { name: "单元B · 执行中" }));
    expect(onSelectWorkItem).toHaveBeenCalledWith("wi_b");
    await user.keyboard("{Tab}");
    await user.keyboard("{Enter}");
    expect(onSelectWorkItem).toHaveBeenCalledTimes(2);
  });

  it("pulses awaiting_triage nodes with the shared reduced-motion-safe class", () => {
    renderGraph();
    const node = screen.getByTestId("coding-topology-graph").querySelector('[data-node="wi_c"]');
    expect(node?.classList.contains("aria-pulse")).toBe(true);
  });

  // F-55 重影回归：节点卡片描边只落在 rect 上，文字不得继承 stroke——
  // 否则 10px 小字每个笔画被描一圈浅色边，呈现「偏移重影」。
  it("keeps the card stroke off node text so captions render without a ghost outline", () => {
    renderGraph();
    const graph = screen.getByTestId("coding-topology-graph");
    const texts = graph.querySelectorAll('[data-node] text');
    expect(texts.length).toBeGreaterThan(0);
    texts.forEach((text) => {
      expect(text).toHaveAttribute("stroke", "none");
    });
    expect(graph.querySelector('[data-node="wi_b"] rect')).toHaveAttribute(
      "stroke",
      "var(--aria-topo-node-running-border)",
    );
  });

  // F-55 对比度：副标题（WI 号 · 状态）用 slate-600 小字，压浅底仍可读。
  it("renders the node subtitle caption in the higher-contrast slate-600 token", () => {
    renderGraph();
    const graph = screen.getByTestId("coding-topology-graph");
    const subtitle = graph.querySelectorAll('[data-node="wi_b"] text')[1];
    expect(subtitle).toHaveTextContent("wi_b · 执行中");
    expect(subtitle?.classList.contains("fill-slate-600")).toBe(true);
  });

  it("renders an explicit empty state when there is no unit", () => {
    render(
      <CodingTopologyGraph topology={{ nodes: [], edges: [] }} selectedWorkItemId={null} onSelectWorkItem={() => undefined} />,
    );
    expect(screen.getByTestId("coding-topology-empty")).toHaveTextContent("暂无执行单元");
  });
});
