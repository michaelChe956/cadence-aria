import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { HumanPresentationRevision } from "../api/types";
import { HumanPresentationEditor } from "../components/workspace/HumanPresentationEditor";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  MockWebSocket,
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";

const DISCONNECTED_SAVE_ERROR = "连接已断开，请重连后重试";

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

  it.each(["close", "error"] as const)(
    "releases every pending presentation save on websocket %s without overwriting settled bundles",
    (disconnectEvent) => {
      const harness = renderWorkspaceHook();
      render(<PresentationSaveState bundleId="plan-projection-001" />);

      act(() => {
        harness.ws.open();
        sendPresentation(harness.api, "plan-projection-001", "plan");
        sendPresentation(harness.api, "work-item-projection-001", "work_item");
        settleNonPendingBundles();
      });

      expect(screen.getByRole("form", { name: "编辑人工说明" })).toHaveAttribute(
        "aria-busy",
        "true",
      );
      expect(screen.getByRole("button", { name: "保存中…" })).toBeDisabled();

      act(() => {
        if (disconnectEvent === "close") {
          harness.ws.close(1006);
        } else {
          harness.ws.onerror?.(new Event("error"));
        }
      });

      expectPendingSavesReleased();
      expectSettledBundlesPreserved();
      expect(screen.getByRole("form", { name: "编辑人工说明" })).toHaveAttribute(
        "aria-busy",
        "false",
      );
      expect(screen.getByRole("button", { name: "保存说明" })).toBeEnabled();
      expect(screen.getByRole("alert")).toHaveTextContent(DISCONNECTED_SAVE_ERROR);
    },
  );

  it("releases pending presentation saves when a replacement connection times out", () => {
    vi.useFakeTimers();
    const firstHarness = renderWorkspaceHook();
    render(<PresentationSaveState bundleId="plan-projection-001" />);

    act(() => {
      firstHarness.ws.open();
      sendPresentation(firstHarness.api, "plan-projection-001", "plan");
      sendPresentation(firstHarness.api, "work-item-projection-001", "work_item");
      settleNonPendingBundles();
    });
    expect(screen.getByRole("button", { name: "保存中…" })).toBeDisabled();

    firstHarness.unmount();
    renderWorkspaceHook();
    expect(MockWebSocket.instances).toHaveLength(2);

    act(() => {
      vi.advanceTimersByTime(5_000);
    });

    expectPendingSavesReleased();
    expectSettledBundlesPreserved();
    expect(screen.getByRole("button", { name: "保存说明" })).toBeEnabled();
    expect(screen.getByRole("alert")).toHaveTextContent(DISCONNECTED_SAVE_ERROR);
  });
});

function PresentationSaveState({ bundleId }: { bundleId: string }) {
  const saveState = useWorkspaceStore(
    (state) => state.humanPresentationSaveStates[bundleId],
  );
  return (
    <HumanPresentationEditor
      base={{
        scope: "plan",
        source_projection_bundle_id: bundleId,
        human_summary: "原始拆分说明",
        why_split: null,
        dependency_explanation: [],
        risk_explanation: [],
        source_refs: [],
        presentation: null,
      }}
      onSave={() => undefined}
      saving={saveState?.saving ?? false}
      error={saveState?.error ?? null}
    />
  );
}

function sendPresentation(
  api: ReturnType<typeof renderWorkspaceHook>["api"],
  bundleId: string,
  scope: "plan" | "work_item",
) {
  api.sendHumanPresentationRevision({
    type: "save_human_presentation_revision",
    source_projection_bundle_id: bundleId,
    scope,
    supersedes: null,
    human_summary: "更清楚的说明",
    why_split: null,
    dependency_explanation: [],
    risk_explanation: [],
    source_refs: [],
  });
}

function settleNonPendingBundles() {
  const store = useWorkspaceStore.getState();
  store.completeHumanPresentationSave(presentationRevision("settled-success"));
  store.failHumanPresentationSave("settled-failure", "原有保存错误");
}

function presentationRevision(bundleId: string): HumanPresentationRevision {
  return {
    id: `human-presentation-${bundleId}`,
    source_plan_projection_bundle_id: bundleId,
    source_work_item_projection_bundle_id: null,
    supersedes: null,
    human_summary: "已保存说明",
    why_split: null,
    dependency_explanation: [],
    risk_explanation: [],
    source_refs: [],
    normative: false,
    used_by_provider: false,
    created_at: "2026-07-18T00:00:00Z",
  };
}

function expectPendingSavesReleased() {
  const states = useWorkspaceStore.getState().humanPresentationSaveStates;
  expect(states["plan-projection-001"]).toEqual({
    saving: false,
    error: DISCONNECTED_SAVE_ERROR,
  });
  expect(states["work-item-projection-001"]).toEqual({
    saving: false,
    error: DISCONNECTED_SAVE_ERROR,
  });
}

function expectSettledBundlesPreserved() {
  const states = useWorkspaceStore.getState().humanPresentationSaveStates;
  expect(states["settled-success"]).toEqual({ saving: false, error: null });
  expect(states["settled-failure"]).toEqual({
    saving: false,
    error: "原有保存错误",
  });
}
