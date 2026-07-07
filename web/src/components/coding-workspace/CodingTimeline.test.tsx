import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodingTimelineNode } from "../../api/types";
import { CodingTimeline } from "./CodingTimeline";

describe("CodingTimeline", () => {
  it("labels internal_pr_review stage as GroupFinalReview", () => {
    render(
      <CodingTimeline
        nodes={[
          timelineNode({
            id: "coding_node_group_final_review",
            stage: "internal_pr_review",
            title: "Internal PR Review",
          }),
        ]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const timeline = screen.getByTestId("coding-timeline");
    expect(timeline).toHaveTextContent("GroupFinalReview");
    expect(timeline).not.toHaveTextContent("Internal PR Review");
  });
});

function timelineNode(overrides: Partial<CodingTimelineNode> = {}): CodingTimelineNode {
  return {
    id: "coding_node_0001",
    attempt_id: "coding_attempt_0001",
    stage: "coding",
    title: "Coding",
    status: "completed",
    agent_role: "system",
    summary: null,
    started_at: "2026-07-07T00:00:00Z",
    completed_at: null,
    artifact_refs: [],
    ...overrides,
  };
}
