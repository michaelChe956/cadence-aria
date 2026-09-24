import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fetchWorkspaceNodeDetail } from "../api/workspace-content";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { makeNodeDetail } from "../state/workspace-ws-store.test-utils";
import type {
  TimelineNode,
  TimelineNodeStatus,
} from "../state/workspace-ws-store-types";
import { useCockpitNodeDetailHydration } from "./useCockpitNodeDetailHydration";

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceNodeDetail: vi.fn(),
}));

const SESSION_ID = "session_hydration";

function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "timeline_node_001",
    node_type: "author_run",
    agent: "pi",
    stage: "running",
    round: null,
    status: "completed",
    title: "Author Run",
    summary: null,
    started_at: "2026-09-23T16:15:05.965Z",
    completed_at: "2026-09-23T16:16:44.926Z",
    duration_ms: 98_961,
    artifact_ref: null,
    provider_config_snapshot: { author: "pi", reviewer: "kimi_code", review_rounds: 1 },
    ...overrides,
  };
}

function fetchedNodeIds(): string[] {
  return vi
    .mocked(fetchWorkspaceNodeDetail)
    .mock.calls.map(([, nodeId]) => nodeId)
    .sort();
}

function renderHydration(nodes: TimelineNode[], activeNodeId: string | null = null) {
  useWorkspaceStore.setState({ sessionId: SESSION_ID, timelineNodes: nodes });
  return renderHook(
    ({ timelineNodes }: { timelineNodes: TimelineNode[] }) =>
      useCockpitNodeDetailHydration({ sessionId: SESSION_ID, timelineNodes, activeNodeId }),
    { initialProps: { timelineNodes: nodes } },
  );
}

describe("useCockpitNodeDetailHydration", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
    vi.mocked(fetchWorkspaceNodeDetail).mockReset();
    vi.mocked(fetchWorkspaceNodeDetail).mockImplementation(async (_sessionId, nodeId) =>
      makeNodeDetail({ node_id: nodeId, session_id: SESSION_ID }),
    );
  });

  // F-47 REQ-NDR-02：水合集合此前只含 completed（+active/selected），failed/skipped
  // 等终态节点永远不被水合 → 即便 durable 有 usage，token 行也永不显示。
  it("hydrates settled nodes beyond completed (failed/skipped/active)", async () => {
    const nodes = [
      timelineNode({ node_id: "n_completed", status: "completed" }),
      timelineNode({ node_id: "n_failed", status: "failed" }),
      timelineNode({ node_id: "n_skipped", status: "skipped" }),
      timelineNode({ node_id: "n_active", status: "active" }),
    ];

    renderHydration(nodes, "n_active");

    await waitFor(() => {
      expect(fetchedNodeIds()).toEqual(["n_active", "n_completed", "n_failed", "n_skipped"]);
    });
  });

  // spec 点名 aborted/interrupted：前端 union 尚未枚举这些字面量，判定必须用
  // 「非 active/paused」排除法而非枚举 completed，否则同形数据再次落在集合外。
  it("hydrates terminal literals the frontend status union does not enumerate yet", async () => {
    const nodes = [
      timelineNode({ node_id: "n_aborted", status: "aborted" as TimelineNodeStatus }),
      timelineNode({ node_id: "n_interrupted", status: "interrupted" as TimelineNodeStatus }),
    ];

    renderHydration(nodes);

    await waitFor(() => {
      expect(fetchedNodeIds()).toEqual(["n_aborted", "n_interrupted"]);
    });
  });

  // 按需语义不变：只拉「呈现节点」（传入的 timelineNodes），不做全量预取；
  // paused 未终态（durable detail 尚未产生）不拉。
  it("fetches only rendered settled nodes and never prefetches unrendered ones", async () => {
    const rendered = [
      timelineNode({ node_id: "n_completed", status: "completed" }),
      timelineNode({ node_id: "n_paused", status: "paused" }),
    ];
    const notRendered = timelineNode({ node_id: "n_not_rendered", status: "completed" });
    // 会话里还有一个节点，但它不在这份呈现列表里（未渲染即不拉）。
    useWorkspaceStore.setState({ timelineNodes: [...rendered, notRendered] });

    renderHydration(rendered);
    await waitFor(() => {
      expect(fetchedNodeIds()).toEqual(["n_completed"]);
    });
    expect(fetchedNodeIds()).not.toContain("n_not_rendered");
  });

  it("keeps a failed node's hydrated detail so its token usage becomes visible", async () => {
    const failedNode = timelineNode({ node_id: "n_failed", status: "failed" });
    vi.mocked(fetchWorkspaceNodeDetail).mockResolvedValue(
      makeNodeDetail({
        node_id: "n_failed",
        session_id: SESSION_ID,
        status: "failed",
        streaming_content: "author 正文",
        execution_events: [
          {
            event_id: "usage_author",
            kind: "usage",
            status: "completed",
            title: "author token usage",
            output: JSON.stringify({ role: "author", input_tokens: 7_597, output_tokens: 19_493 }),
          },
        ],
      }),
    );

    renderHydration([failedNode]);

    await waitFor(() => {
      expect(useWorkspaceStore.getState().nodeDetails.n_failed?.execution_events).toHaveLength(1);
    });
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.metadata?.usage !== undefined),
    ).toBe(true);
  });

  it("does not re-fetch a node whose detail was already hydrated", async () => {
    const nodes = [timelineNode({ node_id: "n_completed", status: "completed" })];
    vi.mocked(fetchWorkspaceNodeDetail).mockResolvedValue(
      makeNodeDetail({ node_id: "n_completed", session_id: SESSION_ID }),
    );

    const { rerender } = renderHydration(nodes);
    await waitFor(() => {
      expect(fetchedNodeIds()).toEqual(["n_completed"]);
    });

    // 同一批节点换数组身份（快照到达即换身份）→ 去重 ref 阻止二次拉取。
    rerender({ timelineNodes: [...nodes] });
    await waitFor(() => {
      expect(useWorkspaceStore.getState().nodeDetails.n_completed).toBeDefined();
    });
    expect(fetchWorkspaceNodeDetail).toHaveBeenCalledTimes(1);
  });
});
