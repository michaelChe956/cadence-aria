// REQ-WSC-27（gate-card-resolved-and-inflight-state，D2）：刷新 / WS 重连后 store 把
// humanGateTurn 置 null，门卡此前退化成「全新可操作的分诊卡」（修订进度提示消失，
// 此时提交会被后端判 Busy）。在途态改由 activeRunId + 活动节点类型派生。
import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import { timelineNode } from "../../../state/workspace-cockpit-projection.test-fixtures";
import { GatePromptEntry } from "./GatePromptEntry";

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
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    sendBootstrapAction: vi.fn(),
  };
}

/** 刷新后的形态：快照在场、humanGateTurn 为 null、门卡 gate_identity = 投影 key。 */
function refreshedGateEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  const key = `stage:human_confirm`;
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-11-07T06:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: null,
      gate_identity: key,
    },
    ...overrides,
  };
}

function seedRefreshedStore(
  nodeType: "author_run" | "reviewer_run",
  extra: Record<string, unknown> = {},
) {
  useWorkspaceStore.setState({
    workspaceType: "work_item_plan",
    flowKind: "single_candidate",
    singleCandidatePhase: "approval",
    stage: "human_confirm",
    sessionStatus: "running",
    humanGateSnapshot: null,
    humanGateTurn: null,
    humanGateClosure: null,
    activeRunId: "run-9",
    activeNodeId: "timeline_node_009",
    timelineNodes: [
      timelineNode({
        node_id: "timeline_node_009",
        node_type: nodeType,
        status: "active",
      }),
    ],
    ...extra,
  });
}

describe("REQ-WSC-27 刷新后在途态不退化为全新可操作卡", () => {
  installWorkspaceStoreTestHooks();

  it("修订中：状态行「已提交，正在按反馈修订」，无反馈框与确认按钮，仍可终止", () => {
    seedRefreshedStore("author_run");

    render(<GatePromptEntry entry={refreshedGateEntry()} actions={actions()} />);

    expect(screen.getByTestId("gate-revision-status")).toHaveTextContent(
      "已提交，正在按反馈修订",
    );
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.getByRole("button", { name: "终止此门" })).toBeVisible();
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "引擎正在按你的反馈修订，请稍候；完成后会自动复评",
    );
    expect(screen.getByTestId("gate-next-step").textContent).not.toContain("确认当前版本");
  });

  it("复评中：文案区别于修订，同样无反馈框与确认按钮", () => {
    seedRefreshedStore("reviewer_run");

    render(<GatePromptEntry entry={refreshedGateEntry()} actions={actions()} />);

    const status = screen.getByTestId("gate-revision-status");
    expect(status).toHaveTextContent("修订完成，正在复评");
    expect(status.textContent).not.toContain("正在按反馈修订");
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "引擎正在复评修订后的版本，请稍候；结论出来后再决定",
    );
  });

  it("run 结束（节点终态）后自动解锁，不被陈旧 activeRunId 永久锁死", () => {
    seedRefreshedStore("author_run");
    render(<GatePromptEntry entry={refreshedGateEntry()} actions={actions()} />);
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();

    act(() =>
      useWorkspaceStore
        .getState()
        .updateTimelineNode("timeline_node_009", "completed", null, "2026-11-07T06:05:00Z"),
    );

    expect(screen.queryByTestId("gate-revision-status")).toBeNull();
    expect(screen.getByTestId("gate-feedback-editor")).toBeVisible();
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeVisible();
  });

  it("已收口卡不被在途态重新激活：会话已终止但陈旧 activeRunId + active 节点仍在", () => {
    seedRefreshedStore("author_run", { sessionStatus: "terminated" });

    render(
      <GatePromptEntry
        entry={refreshedGateEntry({ resolved: true, resolution: "terminate" })}
        actions={actions()}
      />,
    );

    expect(screen.queryByTestId("gate-revision-status")).toBeNull();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.getByText("已终止").tagName).toBe("SPAN");
  });

  it("活动节点类型不是 author_run / reviewer_run 时不凭 activeRunId 单独上锁", () => {
    seedRefreshedStore("author_run", {
      timelineNodes: [
        timelineNode({ node_id: "timeline_node_009", node_type: "human_confirm", status: "active" }),
      ],
    });

    render(<GatePromptEntry entry={refreshedGateEntry()} actions={actions()} />);

    expect(screen.queryByTestId("gate-revision-status")).toBeNull();
    expect(screen.getByTestId("gate-feedback-editor")).toBeVisible();
  });

  it("activeRunId 为空时维持既有可操作形态（回归）", () => {
    seedRefreshedStore("author_run", { activeRunId: null });

    render(<GatePromptEntry entry={refreshedGateEntry()} actions={actions()} />);

    expect(screen.queryByTestId("gate-revision-status")).toBeNull();
    expect(screen.getByTestId("gate-feedback-editor")).toBeVisible();
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeVisible();
  });

  it("live-turn 路径不变：open turn 时编辑器仍可见（既有断言）", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    render(
      <GatePromptEntry
        entry={refreshedGateEntry({
          metadata: {
            action_facade: "typed",
            command_id: "cmd_1",
            action_block_reason: null,
            gate_identity: "turn_1",
            turn_id: "turn_1",
          },
        })}
        actions={actions()}
      />,
    );

    expect(screen.getByTestId("gate-revision-status")).toHaveTextContent(
      "已提交，正在按反馈修订",
    );
    expect(screen.getByTestId("gate-feedback-editor")).toBeVisible();
  });

});
