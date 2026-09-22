import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import {
  handleWorkspaceWsMessage,
  type WsServerMessage,
} from "../../../hooks/workspace-ws-message-handler";
import { GatePromptEntry } from "./GatePromptEntry";

function gateEntry(
  actionBlockReason: "terminal_stage" | "phase_mismatch" | null,
  gateIdentity?: string,
): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-09-17T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: actionBlockReason,
      ...(gateIdentity ? { gate_identity: gateIdentity } : {}),
    },
  };
}

function actions(): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    confirmReview: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
    adoptReview: vi.fn(() => true),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
  };
}


describe("GatePromptEntry actionability", () => {
  installWorkspaceStoreTestHooks();
  it("replaces every gate action with its block reason when the projection is stale", () => {
    const gateActions = actions();

    render(<GatePromptEntry entry={gateEntry("terminal_stage")} actions={gateActions} />);

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
    expect(gateActions.terminate).not.toHaveBeenCalled();
  });
  it("locks a rebuilt legacy gate card when the live stage leaves human confirmation", () => {
    const gateActions = actions();
    useWorkspaceStore.getState().setStage("human_confirm");

    render(
      <GatePromptEntry
        entry={gateEntry(null, "stage:human_confirm")}
        actions={gateActions}
      />,
    );
    expect(screen.getByRole("button", { name: "确认产物" })).toBeVisible();

    act(() => useWorkspaceStore.getState().setStage("compile_plan"));

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
  });
  it("locks a rebuilt typed gate card when the live stage leaves human confirmation", () => {
    const gateActions = actions();
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);

    render(
      <GatePromptEntry
        entry={gateEntry(null, "turn_1")}
        actions={gateActions}
      />,
    );
    expect(screen.getByRole("button", { name: "确认产物" })).toBeVisible();

    act(() =>
      handleWorkspaceWsMessage(
        { type: "stage_change", stage: "compile_plan" } as WsServerMessage,
        {
          invalidatedPreStageNodeIds: new Set<string>(),
          scheduleFlush: vi.fn(),
          streamFlushTimeouts: {},
        },
      ),
    );

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
  });

  // F-38：确认卡此前只说「等待人工确认」——确认者不知道在确认什么。有产物版本时
  // 说明区露出「待确认产物：<种类> vN」+「查看产物」入口；无版本则不猜（fail-closed）。
  describe("F-38 pending artifact entry", () => {
    function artifactVersion(versionNo: number, isCurrent: boolean) {
      return {
        version: versionNo,
        generated_by: "pi" as const,
        reviewed_by: null,
        review_verdict: null,
        confirmed_by: null,
        is_current: isCurrent,
        created_at: "2026-09-22T16:02:09Z",
        source_node_id: `timeline_node_00${versionNo}`,
      };
    }

    it("names the pending artifact and opens the artifact view from the gate card", async () => {
      const gateActions = actions();
      const onOpenArtifact = vi.fn();
      const user = userEvent.setup();
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(1, false), artifactVersion(2, true)],
      });

      render(
        <GatePromptEntry
          entry={gateEntry(null)}
          actions={gateActions}
          onOpenArtifact={onOpenArtifact}
        />,
      );

      expect(screen.getByTestId("gate-artifact-context")).toHaveTextContent(
        "待确认产物：Work Item Plan v2",
      );
      await user.click(screen.getByRole("button", { name: "查看产物" }));
      expect(onOpenArtifact).toHaveBeenCalledOnce();
      expect(gateActions.confirm).not.toHaveBeenCalled();
    });

    it("falls back to the latest version when no version is flagged current", () => {
      useWorkspaceStore.setState({
        workspaceType: "story",
        artifactVersions: [artifactVersion(1, false), artifactVersion(2, false)],
      });

      render(
        <GatePromptEntry entry={gateEntry(null)} actions={actions()} onOpenArtifact={vi.fn()} />,
      );

      expect(screen.getByTestId("gate-artifact-context")).toHaveTextContent(
        "待确认产物：Story Spec v2",
      );
    });

    it("renders no artifact entry when the session has no artifact version", () => {
      useWorkspaceStore.setState({ workspaceType: "work_item_plan", artifactVersions: [] });

      render(
        <GatePromptEntry entry={gateEntry(null)} actions={actions()} onOpenArtifact={vi.fn()} />,
      );

      expect(screen.queryByTestId("gate-artifact-context")).toBeNull();
      expect(screen.queryByRole("button", { name: "查看产物" })).toBeNull();
    });

    it("keeps the artifact entry off cards that have no artifact view to open", () => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(1, true)],
      });

      render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

      expect(screen.queryByTestId("gate-artifact-context")).toBeNull();
    });

    it("drops the pending artifact line once the gate is resolved", () => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(1, true)],
      });

      render(
        <GatePromptEntry
          entry={{ ...gateEntry(null), resolved: true, resolution: "confirm" }}
          actions={actions()}
          onOpenArtifact={vi.fn()}
        />,
      );

      expect(screen.queryByTestId("gate-artifact-context")).toBeNull();
      expect(screen.getByText("已确认")).toBeVisible();
    });
  });

  it("keeps typed gate controls wired to the supplied facade when no block reason exists", async () => {
    const gateActions = actions();
    const user = userEvent.setup();

    render(<GatePromptEntry entry={gateEntry(null)} actions={gateActions} />);

    await user.type(screen.getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(screen.getByRole("button", { name: "提交反馈" }));
    fireEvent.click(screen.getByRole("button", { name: "确认产物" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));
    fireEvent.click(screen.getByRole("button", { name: "确认终止" }));

    expect(gateActions.feedback).toHaveBeenCalledWith("请补齐边界");
    expect(gateActions.confirm).toHaveBeenCalledOnce();
    expect(gateActions.terminate).toHaveBeenCalledOnce();
  });

  // F-21（v28 监控 0449）：plan 会话停在 human_confirm 的 context blocker 门
  //（prepare 相位）——actionBlockReason=phase_mismatch 时终止仍必须露出并接
  // facade.terminate（confirm/反馈编辑器维持相位纪律不渲染）。
  it("renders a terminate-only plan gate card for a phase-mismatched context blocker gate (F-21)", async () => {
    const gateActions = actions();
    const user = userEvent.setup();
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "prepare",
      sessionStatus: "waiting_for_human",
      humanGateClosure: null,
    });

    render(
      <GatePromptEntry
        entry={gateEntry("phase_mismatch", "stage:human_confirm")}
        actions={gateActions}
      />,
    );

    expect(screen.getByRole("button", { name: "终止" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();

    await user.click(screen.getByRole("button", { name: "终止" }));
    expect(gateActions.terminate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "确认终止" }));
    expect(gateActions.terminate).toHaveBeenCalledOnce();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
  });
});
