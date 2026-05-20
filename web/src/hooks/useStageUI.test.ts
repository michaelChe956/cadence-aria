import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useStageUI } from "./useStageUI";

describe("useStageUI", () => {
  it("returns PrepareContextPanel for prepare_context", () => {
    const { result } = renderHook(() => useStageUI("prepare_context"));

    expect(result.current.panel).toBe("PrepareContextPanel");
    expect(result.current.actions).toEqual(["start_generation"]);
    expect(result.current.showContextInput).toBe(true);
    expect(result.current.providerEditable).toBe(true);
  });

  it("returns RunningPanel for running", () => {
    const { result } = renderHook(() => useStageUI("running"));

    expect(result.current.panel).toBe("RunningPanel");
    expect(result.current.actions).toEqual(["abort"]);
    expect(result.current.showContextInput).toBe(false);
    expect(result.current.providerEditable).toBe(false);
  });

  it("returns HumanConfirmPanel for human_confirm", () => {
    const { result } = renderHook(() => useStageUI("human_confirm"));

    expect(result.current.panel).toBe("HumanConfirmPanel");
    expect(result.current.actions).toEqual(["confirm", "request_change", "terminate"]);
    expect(result.current.headerBadge).toBe("等待确认");
  });

  it("returns correct config for all stages", () => {
    const stages = [
      ["prepare_context", "PrepareContextPanel", ["start_generation"]],
      ["running", "RunningPanel", ["abort"]],
      ["cross_review", "CrossReviewPanel", ["abort"]],
      ["review_decision", "ReviewDecisionPanel", ["select_revision_path", "abort"]],
      ["revision", "RevisionPanel", ["abort"]],
      ["human_confirm", "HumanConfirmPanel", ["confirm", "request_change", "terminate"]],
      ["completed", "CompletedPanel", []],
    ] as const;

    for (const [stage, panel, actions] of stages) {
      const { result } = renderHook(() => useStageUI(stage));

      expect(result.current.panel).toBe(panel);
      expect(result.current.actions).toEqual(actions);
    }
  });

  it("falls back to prepare context config for unknown stages", () => {
    const { result } = renderHook(() => useStageUI("unexpected"));

    expect(result.current.panel).toBe("PrepareContextPanel");
    expect(result.current.providerEditable).toBe(true);
  });
});
