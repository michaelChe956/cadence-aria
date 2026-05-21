import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TimelineNode } from "../../state/workspace-ws-store";
import { TimelineNodeList } from "./TimelineNodeList";

describe("TimelineNodeList", () => {
  it("renders timeline nodes with status and selects a node", () => {
    const onSelectNode = vi.fn();

    render(
      <TimelineNodeList
        nodes={[
          timelineNode({ node_id: "note-1", node_type: "context_note", title: "补充上下文", status: "completed" }),
          timelineNode({ node_id: "run-1", node_type: "author_run", title: "Story Spec 生成", status: "active" }),
        ]}
        activeNodeId="run-1"
        selectedNodeId="note-1"
        onSelectNode={onSelectNode}
      />,
    );

    expect(screen.getByTestId("timeline-node-context_note")).toHaveTextContent("补充上下文");
    expect(screen.getByTestId("timeline-node-context_note")).toHaveTextContent("✓");
    expect(screen.getByTestId("timeline-node-author_run")).toHaveTextContent("active");

    fireEvent.click(screen.getByTestId("timeline-node-author_run"));

    expect(onSelectNode).toHaveBeenCalledWith("run-1");
  });

  it("renders an empty state", () => {
    render(
      <TimelineNodeList
        nodes={[]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    expect(screen.getByText("暂无 Timeline 节点")).toBeInTheDocument();
  });
});

function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "author_run",
    agent: "claude_code",
    stage: "running",
    round: null,
    status: "active",
    title: "Story Spec 生成",
    summary: null,
    started_at: "2026-05-21T10:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: {
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    },
    ...overrides,
  };
}
