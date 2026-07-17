import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";

describe("useWorkspaceWs human presentation actions", () => {
  installWorkspaceWsTestHooks();

  it("sends human presentation revisions and exposes recoverable busy state", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanPresentationRevision({
        type: "save_human_presentation_revision",
        source_projection_bundle_id: "plan-projection-001",
        scope: "plan",
        supersedes: null,
        human_summary: "更清楚的说明",
        why_split: null,
        dependency_explanation: [],
        risk_explanation: [],
        source_refs: [],
      });
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "save_human_presentation_revision",
        source_projection_bundle_id: "plan-projection-001",
        scope: "plan",
        supersedes: null,
        human_summary: "更清楚的说明",
        why_split: null,
        dependency_explanation: [],
        risk_explanation: [],
        source_refs: [],
      }),
    ]);
    expect(
      useWorkspaceStore.getState().humanPresentationSaveStates["plan-projection-001"],
    ).toEqual({ saving: true, error: null });
  });
});
