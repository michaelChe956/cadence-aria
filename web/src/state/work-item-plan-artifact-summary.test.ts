import { describe, expect, it } from "vitest";
import type { WorkItemPlanArtifactPayload } from "../api/types";
import { workItemPlanArtifactUpdateSummary } from "./work-item-plan-artifact-summary";

describe("workItemPlanArtifactUpdateSummary", () => {
  it("summarizes context blocker artifacts without treating them as compile reports", () => {
    const artifact: WorkItemPlanArtifactPayload = {
      type: "context_blocker",
      payload: {
        context_blockers: [
          {
            code: "missing_design_context",
            message: "需要补充 Provider 管理入口位置",
            needed_context: ["design_spec_0001"],
          },
        ],
        design_context_gaps: ["缺少入口布局"],
        exploration_summary: "需要人工确认范围",
        allowed_actions: ["provide_context", "terminate"],
      },
    };

    const summary = workItemPlanArtifactUpdateSummary(artifact, 4);

    expect(summary.content).toBe("Context Blocker 已更新 · 1 blockers");
    expect(summary.metadata).toMatchObject({
      version: 4,
      artifact_type: "context_blocker",
      artifact_label: "Context Blocker",
      object_id: "missing_design_context",
      status_label: "blocked",
    });
  });
});
