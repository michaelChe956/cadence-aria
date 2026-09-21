import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ExecutionEvent, TimelineNode, TimelineNodeDetail } from "../../state/workspace-ws-store";
import type { CockpitFlowState } from "../../state/workspace-cockpit-projection";
import { TimelineNodeList } from "./TimelineNodeList";

describe("TimelineNodeList", () => {
  it("renders timeline nodes with status and selects a node", () => {
    const onSelectNode = vi.fn();

    render(
      <TimelineNodeList
        nodes={[
          timelineNode({ node_id: "note-1", node_type: "context_note", title: "补充上下文", status: "completed" }),
          timelineNode({ node_id: "run-1", node_type: "author_run", title: "Story Spec 生成", status: "active" }),
        ]}
        activeNodeId="run-1"
        selectedNodeId="note-1"
        onSelectNode={onSelectNode}
      />,
    );

    expect(screen.getByTestId("timeline-node-context_note")).toHaveTextContent("补充上下文");
    expect(screen.getByTestId("timeline-node-context_note")).toHaveTextContent("✓");
    expect(screen.getByTestId("timeline-node-author_run")).toHaveTextContent("active");

    fireEvent.click(screen.getByTestId("timeline-node-author_run"));

    expect(onSelectNode).toHaveBeenCalledWith("run-1");
  });

  it("renders an empty state", () => {
    render(
      <TimelineNodeList
        nodes={[]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    expect(screen.getByText("暂无 Timeline 节点")).toBeInTheDocument();
  });

  it("labels revision nodes as author rework", () => {
    render(
      <TimelineNodeList
        nodes={[
          timelineNode({
            node_id: "revision-1",
            node_type: "revision",
            agent: "claude_code",
            stage: "revision",
            round: 1,
            title: "返修 Round 1",
            status: "completed",
          }),
        ]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    expect(screen.getByTestId("timeline-node-revision")).toHaveTextContent(
      "Author 返修 Round 1",
    );
  });

  it("renders work item draft nodes with outline id and readable summary", () => {
    render(
      <TimelineNodeList
        nodes={[
          timelineNode({
            node_id: "draft-run-1",
            node_type: "work_item_draft_run",
            title: "Draft · Provider 依赖 HTTP API 端点",
            summary: "outline_backend_api · draft_002 · draft",
            status: "active",
          }),
        ]}
        activeNodeId="draft-run-1"
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const node = screen.getByTestId("timeline-node-work_item_draft_run");
    expect(node).toHaveTextContent("Draft · Provider 依赖 HTTP API 端点");
    expect(node).toHaveTextContent("outline_backend_api");
    expect(node).toHaveTextContent("draft_002");

    const summary = screen.getByText("outline_backend_api · draft_002 · draft");
    expect(summary).toHaveClass("line-clamp-2");
    expect(summary).toHaveClass("break-words");
    expect(summary).not.toHaveClass("truncate");
  });
});

function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "author_run",
    agent: "claude_code",
    stage: "running",
    round: null,
    status: "active",
    title: "Story Spec 生成",
    summary: null,
    started_at: "2026-05-21T10:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: {
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    },
    ...overrides,
  };
}

function nodeDetail(overrides: Partial<TimelineNodeDetail> = {}): TimelineNodeDetail {
  return {
    node_id: "node-flow-1",
    session_id: "session-flow",
    node_type: "author_run",
    status: "active",
    agent_role: "author",
    provider: null,
    messages: [],
    streaming_content: "",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: false,
    base_artifact_ref: null,
    started_at: new Date(2026, 8, 13, 0, 0, 0).toISOString(),
    ended_at: null,
    ...overrides,
  };
}

function usageEvent(payload: Record<string, number>): ExecutionEvent {
  return {
    event_id: `usage-${JSON.stringify(payload)}`,
    kind: "usage",
    status: "completed",
    title: "usage",
    output: JSON.stringify(payload),
  };
}

describe("TimelineNodeList flow variant", () => {
  // 本地时刻构造 → 方块上的挂钟时间断言与运行环境 TZ 无关。
  const startedAt = new Date(2026, 8, 13, 0, 0, 0).toISOString();
  const flowNode = timelineNode({ node_id: "node-flow-1", node_type: "author_run", status: "active" });

  const flowRows = [
    {
      node_id: "node-flow-1",
      title: "Author 运行",
      state: "running" as const,
      index: 1,
      total: 2,
      elapsed_ms: 30_000,
      idle_ms: 5_000,
      started_at: startedAt,
      topology: ["running", "pending"] as const,
    },
    {
      node_id: "node-flow-2",
      title: "Review Round 1",
      state: "awaiting_triage" as const,
      index: 2,
      total: 2,
      elapsed_ms: 90_000,
      idle_ms: 5_000,
      started_at: startedAt,
      topology: ["running", "awaiting_triage"] as const,
    },
  ];

  it("renders the four execution flow essentials per row", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode, timelineNode({ node_id: "node-flow-2", node_type: "reviewer_run" })]}
        activeNodeId="node-flow-1"
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[...flowRows]}
      />,
    );

    expect(screen.getByTestId("flow-status-light-author_run")).toBeInTheDocument();
    expect(screen.getByTestId("flow-progress-author_run")).toHaveTextContent("1/2");
    expect(screen.getByTestId("flow-elapsed-author_run")).toHaveTextContent("30s");
    expect(screen.getAllByTestId("cockpit-mini-topology")).toHaveLength(2);
  });

  it("keeps mono and tabular numeric classes on progress and elapsed", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
      />,
    );

    expect(screen.getByTestId("flow-progress-author_run").className).toContain("aria-mono");
    expect(screen.getByTestId("flow-progress-author_run").className).toContain("aria-num");
    expect(screen.getByTestId("flow-elapsed-author_run").className).toContain("aria-mono");
    expect(screen.getByTestId("flow-elapsed-author_run").className).toContain("aria-num");
  });

  it("exposes no input controls in the flow zone", () => {
    const { container } = render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId="node-flow-1"
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[...flowRows]}
      />,
    );

    expect(container.querySelectorAll("input, select, textarea")).toHaveLength(0);
  });


  it("keeps the sidebar variant free of flow metrics and drills down on click", () => {
    const onSelectNode = vi.fn();
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={onSelectNode}
        flowRows={[...flowRows]}
      />,
    );

    expect(screen.queryByTestId("flow-progress-author_run")).toBeNull();
    expect(screen.queryByTestId("flow-status-light-author_run")).toBeNull();

    fireEvent.click(screen.getByTestId("timeline-node-author_run"));
    expect(onSelectNode).toHaveBeenCalledWith("node-flow-1");
  });

  it("breathes on a running row and stays static on a settled row", () => {
    const settledRow = {
      ...flowRows[0]!,
      node_id: "node-flow-2",
      state: "done" as const,
      elapsed_ms: 12_000,
    };
    render(
      <TimelineNodeList
        nodes={[flowNode, timelineNode({ node_id: "node-flow-2", node_type: "reviewer_run" })]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!, settledRow]}
      />,
    );

    // REQ-UI37-18：running 为主色呼吸灯（aria-pulse）；settled 行静态实心。
    expect(screen.getByTestId("flow-status-light-author_run").className).toContain("aria-pulse");
    expect(screen.getByTestId("flow-status-light-reviewer_run").className).not.toContain(
      "aria-pulse",
    );
  });

  it("mutes a row whose last event is long past and leaves rows with ongoing events unmuted", () => {
    // flowRows[0] 行仍在持续出事件（elapsed 已超 5 分钟，但最近事件刚发生）→ 不得静默；
    // reviewer_run 行同为 running，但自最近事件起已超 5 分钟 → 静默。
    const activeRow = { ...flowRows[0]!, elapsed_ms: 30 * 60_000, idle_ms: 2_000 };
    const quietRow = { ...flowRows[1]!, state: "running" as const, idle_ms: 600_000 };
    render(
      <TimelineNodeList
        nodes={[flowNode, timelineNode({ node_id: "node-flow-2", node_type: "reviewer_run" })]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[activeRow, quietRow]}
      />,
    );

    // REQ-UI37-18：久无事件 → 静默视觉（muted 透明度），不是报警。
    expect(screen.getByTestId("timeline-node-author_run").className).not.toContain("opacity-60");
    expect(screen.getByTestId("timeline-node-reviewer_run").className).toContain("opacity-60");
  });

  it("renders each flow node as a compact block stacked vertically for the narrow left rail", () => {
    render(
      <TimelineNodeList
        nodes={[
          timelineNode({
            node_id: "node-flow-1",
            node_type: "author_run",
            status: "active",
            summary: "outline_backend_api · draft_002",
          }),
        ]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
      />,
    );

    // 左侧窄栏形态：条目自上而下纵向堆叠（不再横向铺网格）；testid 沿用历史名。
    const rail = screen.getByTestId("timeline-flow-grid");
    expect(rail.className).toContain("flex-col");

    // 紧凑条目：摘要不再占条目版面（改由 hover 提示与下钻详情承载）。
    expect(screen.queryByText("outline_backend_api · draft_002")).toBeNull();
  });

  it("shows start time, elapsed time and flow state on the compact block", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
      />,
    );

    expect(screen.getByTestId("flow-started-author_run")).toHaveTextContent("00:00:00");
    expect(screen.getByTestId("flow-started-author_run").className).toContain("aria-mono");
    expect(screen.getByTestId("flow-elapsed-author_run")).toHaveTextContent("30s");
    expect(screen.getByTestId("flow-state-author_run")).toHaveTextContent("运行中");
  });

  it("keeps the full node detail reachable from a compact block", () => {
    const onSelectNode = vi.fn();
    render(
      <TimelineNodeList
        nodes={[
          timelineNode({
            node_id: "node-flow-1",
            node_type: "author_run",
            status: "active",
            summary: "outline_backend_api · draft_002",
          }),
        ]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={onSelectNode}
        variant="flow"
        flowRows={[flowRows[0]!]}
      />,
    );

    const tile = screen.getByTestId("timeline-node-author_run");
    const hint = tile.getAttribute("title") ?? "";
    expect(hint).toContain("Story Spec 生成");
    expect(hint).toContain("运行中");
    expect(hint).toContain("进度 1/2");
    expect(hint).toContain("开始 00:00:00");
    expect(hint).toContain("耗时 30s");
    expect(hint).toContain("outline_backend_api · draft_002");

    // 点击语义不变：下钻到该节点（详情在下钻区域展开）。
    fireEvent.click(tile);
    expect(onSelectNode).toHaveBeenCalledWith("node-flow-1");
  });

  it("shows the node's token usage on the block and in the hover hint", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode, timelineNode({ node_id: "node-flow-2", node_type: "reviewer_run" })]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[...flowRows]}
        nodeDetails={{
          "node-flow-1": nodeDetail({
            execution_events: [usageEvent({ input_tokens: 1_200, output_tokens: 300 })],
          }),
          // 有节点详情但没有 usage 事件 → 不得展示 token。
          "node-flow-2": nodeDetail({ node_id: "node-flow-2", execution_events: [] }),
        }}
      />,
    );

    expect(screen.getByTestId("flow-tokens-author_run")).toHaveTextContent("↘1.2k/300");
    expect(screen.getByTestId("timeline-node-author_run").getAttribute("title")).toContain(
      "Tokens 输入 1,200 · 输出 300",
    );
    expect(screen.queryByTestId("flow-tokens-reviewer_run")).toBeNull();
  });

  it("reads the latest usage event and ignores payloads it cannot parse", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
        nodeDetails={{
          "node-flow-1": nodeDetail({
            execution_events: [
              usageEvent({ input_tokens: 100, output_tokens: 100 }),
              { ...usageEvent({}), output: "not-json" },
              usageEvent({ input_tokens: 2_048, output_tokens: 4_096, cache_read_tokens: 500 }),
              {
                ...usageEvent({}),
                kind: "provider",
                output: JSON.stringify({ input_tokens: 9, output_tokens: 9 }),
              },
            ],
          }),
        }}
      />,
    );

    expect(screen.getByTestId("flow-tokens-author_run")).toHaveTextContent("↘2k/4.1k/500");
  });

  it("keeps the mini topology bounded when the session has many nodes", () => {
    const topology = Array.from({ length: 12 }, (_, index) =>
      index === 11 ? ("running" as const) : ("done" as const),
    ) as CockpitFlowState[];
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[{ ...flowRows[0]!, index: 12, total: 12, topology }]}
      />,
    );

    const dots = Array.from(
      screen.getByTestId("cockpit-mini-topology").children,
    ) as HTMLElement[];
    expect(dots.length).toBeGreaterThan(0);
    expect(dots.length).toBeLessThan(topology.length);
    // 当前步仍在窗口内（且只标记当前步）。
    expect(
      dots.filter((dot) => (dot.getAttribute("style") ?? "").includes("aria-topo-edge-active")),
    ).toHaveLength(1);
    expect(dots.at(-1)?.getAttribute("style")).toContain("aria-topo-edge-active");
  });
});
