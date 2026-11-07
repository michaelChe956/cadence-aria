// REQ-CG-10 前端半边 + REQ-WSC-27 选择器（gate-card-resolved-and-inflight-state）。
import { describe, expect, it, vi } from "vitest";
import { handleWorkspaceWsMessage, type WsServerMessage } from "../hooks/workspace-ws-message-handler";
import {
  hasInterruptedRevisionEvidence,
  isInterruptedRevisionNode,
  selectGateInflightPhase,
} from "./gate-card-state";
import { buildChatEntries } from "./workspace-chat-rebuild";
import { timelineNode } from "./workspace-cockpit-projection.test-fixtures";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";

const INTERRUPTED_SUMMARY = "运行已中止：修订被中断，你的选择未生效";
const INTERRUPTED_SENTENCE = "修订被中断，你的选择未生效";

describe("被中断修订的证据与对话流明说句（REQ-CG-10）", () => {
  installWorkspaceStoreTestHooks();

  const abortedRevisionNode = () =>
    timelineNode({
      node_id: "timeline_node_009",
      node_type: "author_run",
      status: "aborted",
      summary: INTERRUPTED_SUMMARY,
    });

  it("证据判定：aborted 且 summary 含明说句才算", () => {
    expect(isInterruptedRevisionNode(abortedRevisionNode())).toBe(true);
    expect(
      isInterruptedRevisionNode(
        timelineNode({ status: "aborted", summary: "运行已中止" }),
      ),
    ).toBe(false);
    expect(
      isInterruptedRevisionNode(
        timelineNode({ status: "failed", summary: INTERRUPTED_SUMMARY }),
      ),
    ).toBe(false);
    expect(hasInterruptedRevisionEvidence([abortedRevisionNode()])).toBe(true);
    expect(hasInterruptedRevisionEvidence([])).toBe(false);
  });

  it("buildChatEntries 为被中断修订节点追加系统条目，位于该节点已有条目之后", () => {
    useWorkspaceStore.getState().setTimelineNodesForTest([abortedRevisionNode()]);
    const state = useWorkspaceStore.getState();

    const entries = buildChatEntries(state);

    const marker = entries.filter((entry) => entry.content === INTERRUPTED_SENTENCE);
    expect(marker).toHaveLength(1);
    expect(marker[0].role).toBe("system");
    expect(marker[0].node_id).toBe("timeline_node_009");
  });

  it("普通中止（运行已中止）不追加修订句", () => {
    useWorkspaceStore
      .getState()
      .setTimelineNodesForTest([
        timelineNode({ node_id: "n1", node_type: "author_run", status: "aborted", summary: "运行已中止" }),
      ]);

    const entries = buildChatEntries(useWorkspaceStore.getState());

    expect(entries.some((entry) => entry.content === INTERRUPTED_SENTENCE)).toBe(false);
  });

  it("live：timeline_node_updated 帧到达即补条目，重复投递不叠双", () => {
    useWorkspaceStore.getState().setTimelineNodesForTest([
      timelineNode({ node_id: "timeline_node_009", node_type: "author_run", status: "active" }),
    ]);
    const options = {
      invalidatedPreStageNodeIds: new Set<string>(),
      scheduleFlush: vi.fn(),
      streamFlushTimeouts: {},
    };
    const frame = {
      type: "timeline_node_updated",
      node_id: "timeline_node_009",
      status: "aborted",
      summary: INTERRUPTED_SUMMARY,
      completed_at: "2026-11-07T06:11:00Z",
    } as unknown as WsServerMessage;

    handleWorkspaceWsMessage(frame, options);
    handleWorkspaceWsMessage(frame, options);

    const markers = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.content === INTERRUPTED_SENTENCE);
    expect(markers).toHaveLength(1);
  });
});

describe("selectGateInflightPhase（REQ-WSC-27）", () => {
  installWorkspaceStoreTestHooks();

  function seed(overrides: Record<string, unknown> = {}, nodeType = "author_run", nodeStatus = "active") {
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      stage: "human_confirm",
      sessionStatus: "running",
      humanGateTurn: null,
      humanGateClosure: null,
      activeRunId: "run-9",
      activeNodeId: "n9",
      timelineNodes: [
        timelineNode({
          node_id: "n9",
          node_type: nodeType as never,
          status: nodeStatus as never,
        }),
      ],
      ...overrides,
    });
    return useWorkspaceStore.getState();
  }

  it("author_run + active ⇒ revising；reviewer_run + active ⇒ reviewing", () => {
    expect(selectGateInflightPhase(seed())).toBe("revising");
    expect(selectGateInflightPhase(seed({}, "reviewer_run"))).toBe("reviewing");
  });

  it("activeNodeId 缺省时取最后一个 active 节点", () => {
    expect(selectGateInflightPhase(seed({ activeNodeId: null }))).toBe("revising");
  });

  it.each([
    ["activeRunId 为空", { activeRunId: null }],
    ["stage 不是 human_confirm", { stage: "running" }],
    ["flow 不是 single_candidate", { flowKind: "legacy" }],
    ["门已收口", { humanGateClosure: { decision: "terminate", stage: "human_confirm" } }],
    ["会话已终止", { sessionStatus: "terminated" }],
    [
      "存在 live open turn（让位给既有路径）",
      {
        humanGateTurn: {
          turn_id: "t1",
          command_id: "c1",
          remaining_budget: 1,
          status: "open",
          artifact_ref: null,
          failure_class: null,
          failure_message: null,
          opened_at: "2026-11-07T00:00:00Z",
          inlineError: null,
        },
      },
    ],
    [
      "候选编译自动重写 running（让位给其自有文案）",
      {
        candidateCompileFailure: {
          code: "x",
          message: "y",
          auto_rewrite: { phase: "running", remaining: 2 },
        },
      },
    ],
  ])("%s ⇒ null", (_label, overrides) => {
    expect(selectGateInflightPhase(seed(overrides as Record<string, unknown>))).toBeNull();
  });

  it("活动节点终态 / 缺失 / 类型不符 ⇒ null（不凭 activeRunId 单独上锁）", () => {
    expect(selectGateInflightPhase(seed({}, "author_run", "completed"))).toBeNull();
    expect(selectGateInflightPhase(seed({}, "human_confirm"))).toBeNull();
    expect(selectGateInflightPhase(seed({ timelineNodes: [] }))).toBeNull();
  });
});
