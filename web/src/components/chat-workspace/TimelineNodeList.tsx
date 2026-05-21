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

interface TimelineNodeListProps {
  nodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
  className?: string;
}

export function TimelineNodeList({
  nodes,
  activeNodeId,
  selectedNodeId,
  onSelectNode,
  className = "",
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
            />
          ))}
        </div>
      )}
    </nav>
  );
}

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

  return (
    <button
      type="button"
      data-testid={`timeline-node-${node.node_type}`}
      aria-current={active ? "step" : undefined}
      onClick={onSelect}
      className={[
        "block w-full rounded-md border bg-white px-3 py-2 text-left transition-colors",
        selected || active
          ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
          : "border-[var(--aria-line)] hover:border-[var(--aria-primary)]",
      ].join(" ")}
    >
      <div className="flex min-w-0 items-start gap-2">
        <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">
              {node.title}
            </span>
            <span className="inline-flex shrink-0 items-center gap-1 rounded bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--aria-ink-muted)]">
              {completed ? <Check className="h-3 w-3" aria-hidden="true" /> : null}
              {completed ? "✓" : node.status}
            </span>
          </div>
          {node.summary ? (
            <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">{node.summary}</p>
          ) : null}
        </div>
      </div>
    </button>
  );
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
    case "human_confirm":
      return Hand;
    case "aborted_by_disconnect":
    case "protocol_error":
      return AlertTriangle;
    default:
      return Circle;
  }
}
