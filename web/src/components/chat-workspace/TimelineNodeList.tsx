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
import type { TimelineNode } from "../../state/workspace-ws-store";
import {
  formatFlowElapsed,
  topologyTokenName,
  type CockpitFlowRow,
} from "../../state/workspace-cockpit-projection";
import { CockpitMiniTopology } from "./cockpit/CockpitMiniTopology";

interface TimelineNodeListProps {
  nodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
  className?: string;
  variant?: "sidebar" | "flow";
  flowRows?: CockpitFlowRow[];
}

export function TimelineNodeList({
  nodes,
  activeNodeId,
  selectedNodeId,
  onSelectNode,
  className = "",
  variant = "sidebar",
  flowRows,
}: TimelineNodeListProps) {
  return (
    <nav
      aria-label="Timeline 节点"
      data-testid="timeline-node-list"
      className={`min-h-0 overflow-auto bg-[var(--aria-panel-muted)] p-3 ${className}`}
    >
      {nodes.length === 0 ? (
        <div className="rounded-md border border-[var(--aria-line)] bg-white p-3 text-sm text-[var(--aria-ink-muted)]">
          暂无 Timeline 节点
        </div>
      ) : (
        <div className="space-y-2">
          {nodes.map((node) => (
            <TimelineNodeButton
              key={node.node_id}
              node={node}
              active={node.node_id === activeNodeId}
              selected={node.node_id === selectedNodeId}
              onSelect={() => onSelectNode(node.node_id)}
              flowRow={
                variant === "flow"
                  ? (flowRows?.find((row) => row.node_id === node.node_id) ?? null)
                  : null
              }
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

function TimelineNodeButton({
  node,
  active,
  selected,
  onSelect,
  flowRow,
}: {
  node: TimelineNode;
  active: boolean;
  selected: boolean;
  onSelect: () => void;
  flowRow: CockpitFlowRow | null;
}) {
  const Icon = iconForNode(node.node_type);
  const completed = node.status === "completed";
  const title = displayTitleForNode(node);
  // REQ-UI37-18 静默视觉：仍在推进的行超过 5 分钟没有新引擎事件 → 静默（静态透明度）；
  // MUST NOT 报警（报警归四层卡壳体系）。
  const quiet =
    flowRow !== null && flowRow.state === "running" && flowRow.idle_ms >= FLOW_QUIET_AFTER_MS;

  return (
    <button
      type="button"
      data-testid={`timeline-node-${node.node_type}`}
      aria-current={active ? "step" : undefined}
      onClick={onSelect}
      data-flow-quiet={quiet ? "true" : undefined}
      className={[
        "block w-full min-h-11 rounded-md border-2 bg-white px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
        selected || active
          ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
          : "border-[var(--aria-line-strong)] hover:border-[var(--aria-primary)]",
        quiet ? "opacity-60" : "",
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
          {flowRow ? (
            <div className="mt-1 flex min-w-0 items-center justify-between gap-2">
              <span
                data-testid={`flow-status-light-${node.node_type}`}
                aria-hidden="true"
                className={[
                  "h-2.5 w-2.5 shrink-0 rounded-full",
                  FLOW_PULSE_STATES.includes(flowRow.state) ? "aria-pulse" : "",
                ].join(" ")}
                style={{
                  background: `var(--aria-topo-node-${topologyTokenName(flowRow.state)}-fg)`,
                }}
              />
              <span
                data-testid={`flow-progress-${node.node_type}`}
                className="aria-mono aria-num text-[11px] text-[var(--aria-ink-muted)]"
              >
                {flowRow.index}/{flowRow.total}
              </span>
              <span
                data-testid={`flow-elapsed-${node.node_type}`}
                className="aria-mono aria-num text-[11px] text-[var(--aria-ink-muted)]"
              >
                {formatFlowElapsed(flowRow.elapsed_ms)}
              </span>
              <CockpitMiniTopology states={flowRow.topology} currentIndex={flowRow.index - 1} />
            </div>
          ) : null}
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
