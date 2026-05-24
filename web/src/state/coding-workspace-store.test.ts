import { beforeEach, describe, expect, it } from "vitest";
import type {
  CodeReviewReport,
  CodingTimelineNode,
  CodingWsOutMessage,
} from "../api/types";
import { useCodingWorkspaceStore } from "./coding-workspace-store";

const providerConfig = {
  author: "fake" as const,
  reviewer: "fake" as const,
  review_rounds: 1,
};

function codingNode(overrides: Partial<CodingTimelineNode> = {}): CodingTimelineNode {
  return {
    id: "coding_node_0001",
    attempt_id: "coding_attempt_0001",
    stage: "coding",
    title: "代码编写",
    status: "running",
    agent_role: "author",
    summary: null,
    started_at: "2026-05-23T00:00:00Z",
    completed_at: null,
    artifact_refs: [],
    ...overrides,
  };
}

function codeReview(overrides: Partial<CodeReviewReport> = {}): CodeReviewReport {
  return {
    id: "code_review_0001",
    attempt_id: "coding_attempt_0001",
    round: 1,
    verdict: "approve",
    findings: [],
    tested_evidence_refs: [],
    diff_refs: [],
    summary: "review ok",
    created_at: "2026-05-23T00:01:00Z",
    ...overrides,
  };
}

function sessionState(
  overrides: Partial<Extract<CodingWsOutMessage, { type: "coding_session_state" }>> = {},
): Extract<CodingWsOutMessage, { type: "coding_session_state" }> {
  return {
    type: "coding_session_state",
    attempt_id: "coding_attempt_0001",
    status: "running",
    stage: "coding",
    branch_name: "aria/work-items/work_item_0001/attempt-1",
    base_branch: "main",
    worktree_path: "/tmp/worktree",
    rework_count: 0,
    max_auto_rework: 2,
    head_commit: null,
    pushed_remote: null,
    provider_config_snapshot: providerConfig,
    timeline_nodes: [codingNode()],
    active_node_id: "coding_node_0001",
    testing_report: null,
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    ...overrides,
  };
}

describe("coding workspace store", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("initializes attempt state from a websocket session snapshot", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(sessionState({ code_review_reports: [codeReview()] }));

    const state = useCodingWorkspaceStore.getState();
    expect(state.attemptId).toBe("coding_attempt_0001");
    expect(state.status).toBe("running");
    expect(state.stage).toBe("coding");
    expect(state.branchName).toBe("aria/work-items/work_item_0001/attempt-1");
    expect(state.providerConfigSnapshot).toEqual(providerConfig);
    expect(state.timelineNodes).toHaveLength(1);
    expect(state.activeNodeId).toBe("coding_node_0001");
    expect(state.selectedNodeId).toBe("coding_node_0001");
    expect(state.codeReviewReports).toHaveLength(1);
  });

  it("adds and updates timeline nodes while clearing inactive active node", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addTimelineNode(codingNode());

    store.updateTimelineNode("coding_node_0001", "completed", "代码编写完成", "2026-05-23T00:02:00Z");

    const state = useCodingWorkspaceStore.getState();
    expect(state.timelineNodes[0]).toMatchObject({
      status: "completed",
      summary: "代码编写完成",
      completed_at: "2026-05-23T00:02:00Z",
    });
    expect(state.activeNodeId).toBeNull();
  });

  it("deduplicates review reports and stores gate state", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addCodeReviewReport(codeReview({ summary: "old" }));
    store.addCodeReviewReport(codeReview({ summary: "updated" }));
    store.addPendingGate({
      gate_id: "gate_0001",
      kind: "blocked",
      title: "需要人工决策",
      description: "测试失败次数达到上限",
      available_actions: [
        {
          action_id: "accept_risk",
          label: "接受风险",
          action_type: "accept_risk",
        },
      ],
    });

    expect(useCodingWorkspaceStore.getState().codeReviewReports).toEqual([
      codeReview({ summary: "updated" }),
    ]);
    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(1);

    store.resolvePendingGate("gate_0001");

    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(0);
  });

  it("tracks provider streaming content as chat entries", () => {
    const store = useCodingWorkspaceStore.getState();

    store.appendStreamChunk("hello", "coding_node_0001");
    store.appendStreamChunk(" world", "coding_node_0001");

    expect(useCodingWorkspaceStore.getState().streamingContent).toBe("hello world");
    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        type: "provider_stream",
        role: "author",
        content: "hello world",
        node_id: "coding_node_0001",
      },
    ]);

    store.completeStream("coding_node_0001");

    expect(useCodingWorkspaceStore.getState().streamingContent).toBeNull();
    expect(useCodingWorkspaceStore.getState().activeStreamNodeId).toBeNull();
  });

  it("switches artifact tab when selecting timeline nodes until the user locks the tab", () => {
    const store = useCodingWorkspaceStore.getState();
    store.setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "testing" }),
        ],
        active_node_id: null,
      }),
    );

    store.setSelectedNode("coding_node_0002");

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("tests");

    useCodingWorkspaceStore.getState().setActiveTab("logs");
    useCodingWorkspaceStore.getState().setSelectedNode("coding_node_0001");

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("logs");
  });

  it("syncs the artifact tab from the selected snapshot node unless the user locked it", () => {
    const store = useCodingWorkspaceStore.getState();
    store.setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "testing" }),
        ],
        active_node_id: "coding_node_0002",
      }),
    );

    expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0002");
    expect(useCodingWorkspaceStore.getState().activeTab).toBe("tests");

    useCodingWorkspaceStore.getState().setActiveTab("logs");
    useCodingWorkspaceStore.getState().setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "testing" }),
        ],
        active_node_id: "coding_node_0002",
      }),
    );

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("logs");
  });
});
