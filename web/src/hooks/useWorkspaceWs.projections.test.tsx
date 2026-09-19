import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type {
  ArtifactUpdateMessage,
  HumanPresentationRevision,
  PlanProjectionBundle,
  WorkItemProjectionBundle,
} from "../api/types";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";

describe("useWorkspaceWs projection artifacts", () => {
  installWorkspaceWsTestHooks();

  it("assembles live projection updates without markdown fallback", () => {
    const harness = renderWorkspaceHook();
    const workItemProjection = {
      id: "projection-wi-01",
      work_item_revision_id: "revision-wi-01",
    } as WorkItemProjectionBundle;
    const planProjection = {
      id: "projection-plan-01",
      plan_revision_id: "plan-revision-01",
      work_item_projection_bundle_refs: [workItemProjection.id],
    } as PlanProjectionBundle;
    const history = { entries: [] };
    const validation = { findings: [] };

    const updates = [
      {
        type: "artifact_update",
        version: 20,
        work_item_projection: workItemProjection,
      },
      {
        type: "artifact_update",
        version: 21,
        projection_validation: validation,
      },
      {
        type: "artifact_update",
        version: 22,
        work_item_revision_history: history,
      },
      {
        type: "artifact_update",
        version: 23,
        plan_projection: planProjection,
      },
    ] satisfies ArtifactUpdateMessage[];

    act(() => {
      updates.forEach((update) => harness.ws.receive(update));
    });

    const state = useWorkspaceStore.getState();
    const liveVersions = state.workItemPlanArtifactVersions.slice(-4);
    expect(state.artifact).toBeNull();
    expect(liveVersions.map((version) => version.artifact?.type)).toEqual([
      "work_item_projection",
      "projection_validation",
      "work_item_revision_history",
      "plan_projection",
    ]);
    expect(liveVersions.at(-1)?.is_current).toBe(true);
    expect(
      liveVersions.slice(0, -1).every((version) => version.is_current === false),
    ).toBe(true);
    expect(state.workItemPlanProjectionArtifacts).toMatchObject({
      planProjection,
      workItemProjections: [workItemProjection],
      history,
      validation,
      missingWorkItemProjectionRefs: [],
    });
    expect(state.chatEntries.at(-1)).toMatchObject({
      content: "Plan Projection 已发布 · plan-revision-01",
      metadata: expect.objectContaining({ artifact_type: "plan_projection" }),
    });
  });

  it("recovers presentation overlays and resolves save ack and errors by bundle", () => {
    const harness = renderWorkspaceHook();
    const revision = humanPresentationRevision();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_001",
        workspace_type: "work_item_plan",
        stage: "human_confirm",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [],
        active_node_id: null,
        artifact_versions: [],
        artifact_version_summaries: [],
        timeline_node_details: {},
        active_run_id: null,
        human_presentation_revisions: [revision],
      });
    });

    expect(useWorkspaceStore.getState().humanPresentationRevisions).toEqual({
      "plan-projection-001": revision,
    });
    // L2 退役留档（T5/REQ-RET-02）：save ack/error 半段（beginHumanPresentation
    // Save+saved/failed 出站消息）随保存命令族删除——读侧 overlay 恢复断言保留。
  });
});

function humanPresentationRevision(): HumanPresentationRevision {
  return {
    id: "presentation-001",
    source_plan_projection_bundle_id: "plan-projection-001",
    source_work_item_projection_bundle_id: null,
    supersedes: null,
    human_summary: "可读说明",
    why_split: null,
    dependency_explanation: [],
    risk_explanation: [],
    source_refs: [],
    normative: false,
    used_by_provider: false,
    created_at: "2026-07-18T12:00:00Z",
  };
}
