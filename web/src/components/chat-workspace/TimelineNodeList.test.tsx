import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ExecutionEvent, TimelineNode, TimelineNodeDetail } from "../../state/workspace-ws-store";
import { emptyNodeDetail } from "../../state/workspace-ws-store-helpers";
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

/** detail 尚未水合时的真实形状：store 的占位壳（`emptyNodeDetail`）。 */
function placeholderNodeDetail(node: TimelineNode): TimelineNodeDetail {
  return emptyNodeDetail(node.node_id, { sessionId: "session-flow", node });
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

  // F-40：token 三段数与时间/耗时同行时，左侧窄栏方块里会顶穿右边界、尾字被裁。
  // 裁决：token 另起一行（时间/耗时行下方），窄卡不溢出且不截数。
  it("puts the token reading on its own row below the time row", () => {
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
            execution_events: [
              usageEvent({ input_tokens: 1_200, output_tokens: 300, cache_read_tokens: 500 }),
            ],
          }),
          "node-flow-2": nodeDetail({
            node_id: "node-flow-2",
            node_type: "reviewer_run",
            agent_role: "reviewer",
            execution_events: [
              usageEvent({ input_tokens: 7_855, output_tokens: 6_084, cache_read_tokens: 30_592 }),
            ],
          }),
        }}
      />,
    );

    for (const nodeType of ["author_run", "reviewer_run"] as const) {
      const tokenRow = screen.getByTestId(`flow-token-row-${nodeType}`);
      const tokens = screen.getByTestId(`flow-tokens-${nodeType}`);
      const started = screen.getByTestId(`flow-started-${nodeType}`);
      const elapsed = screen.getByTestId(`flow-elapsed-${nodeType}`);
      const state = screen.getByTestId(`flow-state-${nodeType}`);

      // 独立一行：token 读数不得与时间/耗时/状态同行。
      expect(tokenRow.contains(started)).toBe(false);
      expect(tokenRow.contains(elapsed)).toBe(false);
      expect(tokenRow.contains(state)).toBe(false);
      expect(tokens.parentElement).toBe(tokenRow);
      // 时间/耗时行在前，token 行在其下方。
      expect(
        started.compareDocumentPosition(tokenRow) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      // 不溢出且不截数：读数不挂 truncate，行内允许换行吸收极长读数。
      expect(tokens.className).not.toContain("truncate");
      expect(tokenRow.className).toContain("flex-wrap");
    }
  });

  it("renders no token row at all when the node detail carries no usage event", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
        nodeDetails={{ "node-flow-1": nodeDetail({ execution_events: [] }) }}
      />,
    );

    expect(screen.queryByTestId("flow-tokens-author_run")).toBeNull();
    expect(screen.queryByTestId("flow-token-row-author_run")).toBeNull();
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

  // v38 复验 #1：数据面仅 1 个 reviewer_run 节点却渲染两张「Review Round 1」卡。
  // 根因在 store 的 addTimelineNode 盲 push（重连补发窗口重放 created 帧，见
  // workspace-ws-store.test.ts 重放幂等用例）；组件层另按 node_id 去重做防御：
  // 即便上游再引入重复帧（observer 快照、未来新链路），同一 node_id 只渲染一张卡。
  it("renders a node_id once when the input contains replayed duplicate frames", () => {
    const reviewerRun = timelineNode({
      node_id: "timeline_node_004",
      node_type: "reviewer_run",
      stage: "cross_review",
      round: 1,
      title: "Review Round 1",
      status: "active",
    });
    const { unmount } = render(
      <TimelineNodeList
        nodes={[reviewerRun, { ...reviewerRun }]}
        activeNodeId="timeline_node_004"
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
      />,
    );

    expect(screen.getAllByTestId("timeline-node-reviewer_run")).toHaveLength(1);
    unmount();

    render(
      <TimelineNodeList
        nodes={[reviewerRun, { ...reviewerRun }]}
        activeNodeId="timeline_node_004"
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );
    expect(screen.getAllByTestId("timeline-node-reviewer_run")).toHaveLength(1);
  });

  // F-47 REQ-NDR-03：detail 未水合时 token 位此前直接不渲染，用户无法区分
  //「还没读到」与「确实没有 usage」→ 未水合时改为 pending 占位。
  it("shows a pending token slot while the node detail is still unhydrated", () => {
    render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
        nodeDetails={{ "node-flow-1": placeholderNodeDetail(flowNode) }}
      />,
    );

    expect(screen.getByTestId("flow-tokens-pending-author_run")).toHaveTextContent("读取中");
    expect(screen.queryByTestId("flow-tokens-author_run")).toBeNull();
  });

  // Review Focus 3：占位只在 detail 未到时出现——水合一到即转终态（数值或确认缺失），
  // 不得残留 pending（水合极快时占位须一闪即走）。
  it("turns the token slot terminal once the detail arrives", () => {
    const { rerender } = render(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
        nodeDetails={{ "node-flow-1": placeholderNodeDetail(flowNode) }}
      />,
    );
    expect(screen.getByTestId("flow-tokens-pending-author_run")).toBeInTheDocument();

    rerender(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
        nodeDetails={{
          "node-flow-1": nodeDetail({
            execution_events: [usageEvent({ input_tokens: 1_200, output_tokens: 300 })],
          }),
        }}
      />,
    );
    expect(screen.queryByTestId("flow-tokens-pending-author_run")).toBeNull();
    expect(screen.getByTestId("flow-tokens-author_run")).toHaveTextContent("↘1.2k/300");

    // 已水合但无 usage 事件 → 确认缺失终态：既不显示占位也不显示数值。
    rerender(
      <TimelineNodeList
        nodes={[flowNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[flowRows[0]!]}
        nodeDetails={{ "node-flow-1": nodeDetail({ execution_events: [] }) }}
      />,
    );
    expect(screen.queryByTestId("flow-tokens-pending-author_run")).toBeNull();
    expect(screen.queryByTestId("flow-tokens-author_run")).toBeNull();
  });

  // 无 provider 用量位的节点（门卡等）不占位——它们的 detail 本就不产生 usage 事件。
  it("shows no pending token slot for nodes without a provider usage slot", () => {
    const gateNode = timelineNode({
      node_id: "node-flow-2",
      node_type: "human_confirm",
      agent: null,
    });
    render(
      <TimelineNodeList
        nodes={[gateNode]}
        activeNodeId={null}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        variant="flow"
        flowRows={[{ ...flowRows[1]!, node_id: "node-flow-2" }]}
        nodeDetails={{ "node-flow-2": placeholderNodeDetail(gateNode) }}
      />,
    );

    expect(screen.queryByTestId("flow-tokens-pending-human_confirm")).toBeNull();
    expect(screen.queryByTestId("flow-token-row-human_confirm")).toBeNull();
  });
});
