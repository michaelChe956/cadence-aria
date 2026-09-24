// web/src/components/coding-workspace/dashboard/CodingDashboard.test.tsx
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { CodingExecutionUnit } from "../../../api/types";
import { useCodingWorkspaceStore } from "../../../state/coding-workspace-store";
import { CodingDashboard } from "./CodingDashboard";

const unit: CodingExecutionUnit = {
  unit_id: "u1",
  logical_work_item_id: "WI-001",
  work_item_revision_id: "r1",
  dependency_logical_work_item_ids: [],
  order_index: 0,
  status: "running",
  summary: "登录",
  latest_handoff_revision_id: null,
  completion_commit: null,
};

describe("CodingDashboard", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.setState({
      units: [unit],
      timelineNodes: [],
      currentWorkItemId: "WI-001",
    });
  });

  afterEach(() => {
    useCodingWorkspaceStore.setState({ units: [], timelineNodes: [], currentWorkItemId: null });
  });

  // F-55 对比度：「只读视图」副标题小灰字提升到 slate-600，压浅灰底仍可读。
  it("renders the read-only caption in the higher-contrast slate-600 token", () => {
    render(<CodingDashboard />);
    const caption = screen.getByText("只读视图 · 动作入口统一收口于 Phase 4");
    expect(caption.className).toContain("text-slate-600");
  });
});
