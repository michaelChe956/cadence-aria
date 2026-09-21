import {
  AlertTriangle,
  Check,
  Circle,
  Eye,
  Hand,
  MessageCircle,
  Play,
  RefreshCw,
  Bot,
} from "lucide-react";
import type { TimelineNode, TimelineNodeDetail } from "../../state/workspace-ws-store";
import {
  formatFlowElapsed,
  topologyTokenName,
  type CockpitFlowRow,
  type CockpitFlowState,
} from "../../state/workspace-cockpit-projection";
import { elapsedDurationText } from "../shared/duration";
import { CockpitMiniTopology } from "./cockpit/CockpitMiniTopology";

interface TimelineNodeListProps {
  nodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
  className?: string;
  variant?: "sidebar" | "flow";
  flowRows?: CockpitFlowRow[];
  /** 节点详情（同源自 store 的 nodeDetails）：方块据此展示节点级 token 消耗，缺失即不展示。 */
  nodeDetails?: Record<string, TimelineNodeDetail>;
}

export function TimelineNodeList({
  nodes,
  activeNodeId,
  selectedNodeId,
  onSelectNode,
  className = "",
  variant = "sidebar",
  flowRows,
  nodeDetails,
}: TimelineNodeListProps) {
  // v38 复验 #1 防御：重连补发窗口可能把同一节点的 created 帧重复送进数组
  //（store 的 addTimelineNode 已按 node_id 幂等兜底；此处再按 node_id 去重，
  // 首次出现位置优先——同一 node_id 只渲染一张卡，避免 React 同 key 双渲染）。
  const uniqueNodes = nodes.filter(
    (node, index) => nodes.findIndex((candidate) => candidate.node_id === node.node_id) === index,
  );
  return (
    <nav
      aria-label="Timeline 节点"
      data-testid="timeline-node-list"
      className={`min-h-0 overflow-auto bg-[var(--aria-panel-muted)] ${
        variant === "flow" ? "p-2" : "p-3"
      } ${className}`}
    >
      {uniqueNodes.length === 0 ? (
        <div className="rounded-md border border-[var(--aria-line)] bg-white p-3 text-sm text-[var(--aria-ink-muted)]">
          暂无 Timeline 节点
        </div>
      ) : variant === "flow" ? (
        // 左侧窄栏形态（自上而下）：节点条目纵向堆叠成紧凑列表，不再横向铺网格；
        // 完整信息由 hover 提示（title）与点击下钻的详情面承载。
        // （testid「timeline-flow-grid」为既有契约名，沿用不改。）
        <div data-testid="timeline-flow-grid" className="flex flex-col gap-1.5">
          {uniqueNodes.map((node) => (
            <TimelineFlowTile
              key={node.node_id}
              node={node}
              active={node.node_id === activeNodeId}
              selected={node.node_id === selectedNodeId}
              onSelect={() => onSelectNode(node.node_id)}
              flowRow={flowRows?.find((row) => row.node_id === node.node_id) ?? null}
              nodeDetail={nodeDetails?.[node.node_id]}
            />
          ))}
        </div>
      ) : (
        <div className="space-y-2">
          {uniqueNodes.map((node) => (
            <TimelineNodeButton
              key={node.node_id}
              node={node}
              active={node.node_id === activeNodeId}
              selected={node.node_id === selectedNodeId}
              onSelect={() => onSelectNode(node.node_id)}
            />
          ))}
        </div>
      )}
    </nav>
  );
}

// REQ-UI37-18：进行中状态灯带呼吸动画（running 为主色，见 T1 的
// `--aria-topo-node-running-fg: var(--aria-primary)`）；减弱动效下由 T1 的降级块静态实心化。
const FLOW_PULSE_STATES: CockpitFlowRow["state"][] = ["running", "awaiting_triage"];
// 静默判据（M2）：行仍在推进（running）且自最近一次引擎事件（T4 由节点 `last_event_at`
// 推出 `idle_ms`；无该字段时回退 `started_at`）起已超过 5 分钟没有新事件 → 静默视觉
// （静态透明度，非动画）。持续流式事件的长 running 行不得被判为静默。
const FLOW_QUIET_AFTER_MS = 5 * 60_000;
// 方块宽度有限：拓扑小图只渲染当前步附近的一段窗口，避免长会话（上百节点）撑破方块。
const FLOW_TOPOLOGY_WINDOW = 7;
const FLOW_STATE_LABELS: Record<CockpitFlowState, string> = {
  running: "运行中",
  awaiting_triage: "待分诊",
  blocked: "已阻塞",
  failed: "失败",
  pending: "待执行",
  done: "已完成",
};

function TimelineNodeButton({
  node,
  active,
  selected,
  onSelect,
}: {
  node: TimelineNode;
  active: boolean;
  selected: boolean;
  onSelect: () => void;
}) {
  const Icon = iconForNode(node.node_type);
  const completed = node.status === "completed";
  const title = displayTitleForNode(node);

  return (
    <button
      type="button"
      data-testid={`timeline-node-${node.node_type}`}
      aria-current={active ? "step" : undefined}
      onClick={onSelect}
      className={[
        "block w-full min-h-11 rounded-md border-2 bg-white px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
        selected || active
          ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
          : "border-[var(--aria-line-strong)] hover:border-[var(--aria-primary)]",
      ].join(" ")}
    >
      <div className="flex min-w-0 items-start gap-2">
        <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">
              {title}
            </span>
            <span className="aria-chip shrink-0 text-[11px] text-[var(--aria-ink-muted)]">
              {completed ? <Check className="h-3 w-3" aria-hidden="true" /> : null}
              {completed ? "✓" : node.status}
            </span>
          </div>
          {node.summary ? (
            <p className="mt-1 line-clamp-2 break-words text-xs leading-4 text-[var(--aria-ink-muted)]">
              {node.summary}
            </p>
          ) : null}
        </div>
      </div>
    </button>
  );
}

/**
 * 自动执行流的紧凑方块：每个节点一个小方块，只展示状态灯、进度、耗时与开始时刻；
 * 完整信息（摘要 / Provider / 轮次 / 结束时刻）挂在 hover 提示里，点击仍下钻到详情。
 */
function TimelineFlowTile({
  node,
  active,
  selected,
  onSelect,
  flowRow,
  nodeDetail,
}: {
  node: TimelineNode;
  active: boolean;
  selected: boolean;
  onSelect: () => void;
  flowRow: CockpitFlowRow | null;
  nodeDetail?: TimelineNodeDetail;
}) {
  const title = displayTitleForNode(node);
  const stateLabel = flowRow ? FLOW_STATE_LABELS[flowRow.state] : node.status;
  const tokenUsage = nodeTokenUsage(nodeDetail);
  const startedAt = clockTimeText(flowRow?.started_at ?? node.started_at);
  const completedAt = clockTimeText(node.completed_at);
  // 方块的时间读取取 flowRow（与执行流同源）；缺 flowRow 时回退节点自身的耗时。
  const duration = flowRow
    ? formatFlowElapsed(flowRow.elapsed_ms)
    : elapsedDurationText(node.started_at, node.completed_at, node.duration_ms);
  // REQ-UI37-18 静默视觉：仍在推进的行超过 5 分钟没有新引擎事件 → 静默（静态透明度）；
  // MUST NOT 报警（报警归四层卡壳体系）。
  const quiet =
    flowRow !== null && flowRow.state === "running" && flowRow.idle_ms >= FLOW_QUIET_AFTER_MS;
  const topology = flowRow ? topologyWindow(flowRow.topology, flowRow.index - 1) : null;
  const hint = [
    title,
    stateLabel,
    flowRow ? `进度 ${flowRow.index}/${flowRow.total}` : null,
    startedAt ? `开始 ${startedAt}` : null,
    completedAt ? `结束 ${completedAt}` : null,
    duration ? `耗时 ${duration}` : null,
    tokenUsage?.detail ?? null,
    node.agent ? `Provider ${node.agent}` : null,
    node.round === null || node.round === undefined ? null : `Round ${node.round}`,
    node.summary?.trim() ? node.summary.trim() : null,
  ]
    .filter((part): part is string => Boolean(part))
    .join(" · ");

  return (
    <button
      type="button"
      data-testid={`timeline-node-${node.node_type}`}
      aria-current={active ? "step" : undefined}
      title={hint}
      onClick={onSelect}
      data-flow-quiet={quiet ? "true" : undefined}
      className={[
        "flex min-h-[3.5rem] w-full min-w-0 flex-col gap-1 rounded-lg border-2 bg-white px-2 py-1.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
        selected || active
          ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
          : "border-[var(--aria-line-strong)] hover:border-[var(--aria-primary)]",
        quiet ? "opacity-60" : "",
      ].join(" ")}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        {flowRow ? (
          <span
            data-testid={`flow-status-light-${node.node_type}`}
            aria-hidden="true"
            className={[
              "h-2 w-2 shrink-0 rounded-full",
              FLOW_PULSE_STATES.includes(flowRow.state) ? "aria-pulse" : "",
            ].join(" ")}
            style={{
              background: `var(--aria-topo-node-${topologyTokenName(flowRow.state)}-fg)`,
            }}
          />
        ) : null}
        <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-[var(--aria-ink)]">
          {title}
        </span>
      </span>
      {flowRow && topology ? (
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 overflow-hidden">
            <CockpitMiniTopology states={topology.states} currentIndex={topology.currentIndex} />
          </span>
          <span
            data-testid={`flow-progress-${node.node_type}`}
            className="aria-mono aria-num ml-auto shrink-0 text-[10px] text-[var(--aria-ink-muted)]"
          >
            {flowRow.index}/{flowRow.total}
          </span>
        </span>
      ) : null}
      <span className="flex min-w-0 items-center gap-1.5">
        {startedAt ? (
          <time
            data-testid={`flow-started-${node.node_type}`}
            dateTime={flowRow?.started_at ?? node.started_at}
            className="aria-mono aria-num shrink-0 text-[10px] text-[var(--aria-ink-muted)]"
          >
            {startedAt}
          </time>
        ) : null}
        {duration ? (
          <span
            data-testid={`flow-elapsed-${node.node_type}`}
            className="aria-mono aria-num shrink-0 text-[10px] text-[var(--aria-ink-muted)]"
          >
            {duration}
          </span>
        ) : null}
        {tokenUsage ? (
          <span
            data-testid={`flow-tokens-${node.node_type}`}
            aria-label={tokenUsage.detail}
            className="aria-mono aria-num shrink-0 text-[10px] text-[var(--aria-ink-muted)]"
          >
            {tokenUsage.compact}
          </span>
        ) : null}
        <span
          data-testid={`flow-state-${node.node_type}`}
          className="ml-auto min-w-0 truncate text-[10px] text-[var(--aria-ink-muted)]"
        >
          {stateLabel}
        </span>
      </span>
    </button>
  );
}

/** 节点级 token 消耗：方块上的紧凑读数 + hover 提示里的完整读数。 */
interface NodeTokenUsage {
  compact: string;
  detail: string;
}

/**
 * 从节点详情的执行事件里取最近一次 `usage` 负载（与实时链路 `parseUsagePayload` 的字段契约
 * 一致）；没有该事件或负载非法时返回 null——不伪造 token 读数。
 */
function nodeTokenUsage(detail: TimelineNodeDetail | undefined): NodeTokenUsage | null {
  const events = detail?.execution_events;
  if (!events || events.length === 0) {
    return null;
  }
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event?.kind !== "usage" || typeof event.output !== "string" || event.output.length === 0) {
      continue;
    }
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(event.output) as Record<string, unknown>;
    } catch {
      continue;
    }
    const segments: { label: string; value: number }[] = [];
    for (const [label, key] of [
      ["输入", "input_tokens"],
      ["输出", "output_tokens"],
      ["缓存", "cache_read_tokens"],
    ] as const) {
      const raw = parsed[key];
      if (typeof raw === "number" && Number.isFinite(raw) && raw > 0) {
        segments.push({ label, value: raw });
      }
    }
    if (segments.length === 0) {
      return null;
    }
    return {
      // 千位以上折算成 `1.2k`，与方块的小字号版面匹配。
      compact: `↘${segments
        .map(({ value }) =>
          value < 1000 ? String(value) : `${(value / 1000).toFixed(1).replace(/\.0$/, "")}k`,
        )
        .join("/")}`,
      detail: `Tokens ${segments
        .map((segment) => `${segment.label} ${segment.value.toLocaleString("en-US")}`)
        .join(" · ")}`,
    };
  }
  return null;
}

/** 取当前步附近的一段拓扑窗口（窗口不移动时原样返回）。 */
function topologyWindow(topology: readonly CockpitFlowState[], currentIndex: number) {
  if (topology.length <= FLOW_TOPOLOGY_WINDOW) {
    return { states: topology, currentIndex };
  }
  const start = Math.min(
    Math.max(currentIndex - Math.floor(FLOW_TOPOLOGY_WINDOW / 2), 0),
    topology.length - FLOW_TOPOLOGY_WINDOW,
  );
  return {
    states: topology.slice(start, start + FLOW_TOPOLOGY_WINDOW),
    currentIndex: currentIndex - start,
  };
}

/** 本地挂钟 HH:MM:SS；无效/缺失时间返回 null。 */
function clockTimeText(value?: string | null) {
  if (!value) {
    return null;
  }
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return null;
  }
  const date = new Date(parsed);
  return [date.getHours(), date.getMinutes(), date.getSeconds()]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

function displayTitleForNode(node: TimelineNode) {
  if (node.node_type === "revision" && !node.title.startsWith("Author ")) {
    return `Author ${node.title}`;
  }
  return node.title;
}

function iconForNode(nodeType: TimelineNode["node_type"]) {
  switch (nodeType) {
    case "context_note":
      return MessageCircle;
    case "start_generation":
      return Play;
    case "author_run":
      return Bot;
    case "reviewer_run":
      return Eye;
    case "revision":
      return RefreshCw;
    case "author_confirm":
    case "human_confirm":
      return Hand;
    case "aborted_by_disconnect":
    case "protocol_error":
      return AlertTriangle;
    default:
      return Circle;
  }
}
