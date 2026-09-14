// web/src/components/coding-workspace/dashboard/CodingTopologyGraph.tsx
import type { KeyboardEvent } from "react";
import type { CodingTopology, CodingTopologyNode } from "../../../state/coding-dashboard-projection";
import { CODING_UNIT_STATE_LABELS, topologyTokenName } from "../../../state/coding-dashboard-projection";

const NODE_WIDTH = 176;
const NODE_HEIGHT = 44; // R2：节点可点击区域 ≥ 44px
const GAP_X = 64;
const GAP_Y = 12;
const PADDING = 8;

// 分层 = 最长路径深度（依赖无环；有环时按 0 层稳定降级，不抛错）。
function nodeDepths(topology: CodingTopology): Map<string, number> {
  const incoming = new Map<string, string[]>(topology.nodes.map((node) => [node.workItemId, []]));
  for (const edge of topology.edges) {
    incoming.get(edge.toWorkItemId)?.push(edge.fromWorkItemId);
  }
  const depth = new Map<string, number>();
  const resolving = new Set<string>();
  function resolve(id: string): number {
    const known = depth.get(id);
    if (known !== undefined) return known;
    if (resolving.has(id)) return 0;
    resolving.add(id);
    let best = 0;
    for (const from of incoming.get(id) ?? []) {
      best = Math.max(best, resolve(from) + 1);
    }
    resolving.delete(id);
    depth.set(id, best);
    return best;
  }
  for (const node of topology.nodes) resolve(node.workItemId);
  return depth;
}

function layout(topology: CodingTopology) {
  const depths = nodeDepths(topology);
  const layers = new Map<number, CodingTopologyNode[]>();
  for (const node of topology.nodes) {
    const depth = depths.get(node.workItemId) ?? 0;
    layers.set(depth, [...(layers.get(depth) ?? []), node]);
  }
  const positions = new Map<string, { x: number; y: number }>();
  let maxLayerSize = 0;
  for (const [depth, layerNodes] of layers) {
    const ordered = [...layerNodes].sort(
      (left, right) => left.orderIndex - right.orderIndex || left.workItemId.localeCompare(right.workItemId),
    );
    maxLayerSize = Math.max(maxLayerSize, ordered.length);
    ordered.forEach((node, index) => {
      positions.set(node.workItemId, {
        x: PADDING + depth * (NODE_WIDTH + GAP_X),
        y: PADDING + index * (NODE_HEIGHT + GAP_Y),
      });
    });
  }
  const maxDepth = Math.max(0, ...[...layers.keys()]);
  return {
    positions,
    width: PADDING * 2 + (maxDepth + 1) * NODE_WIDTH + maxDepth * GAP_X,
    height: PADDING * 2 + maxLayerSize * NODE_HEIGHT + Math.max(0, maxLayerSize - 1) * GAP_Y,
  };
}

export function CodingTopologyGraph({
  topology,
  selectedWorkItemId,
  onSelectWorkItem,
}: {
  topology: CodingTopology;
  selectedWorkItemId: string | null;
  onSelectWorkItem: (workItemId: string) => void;
}) {
  if (topology.nodes.length === 0) {
    return (
      <div
        data-testid="coding-topology-empty"
        className="flex min-h-11 items-center justify-center rounded-lg border border-dashed border-[var(--aria-line-strong)] px-4 py-3 text-xs text-[var(--aria-ink-muted)]"
      >
        暂无执行单元
      </div>
    );
  }

  const { positions, width, height } = layout(topology);

  function handleKeyDown(event: KeyboardEvent<SVGGElement>, workItemId: string) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelectWorkItem(workItemId);
    }
  }

  return (
    <div data-testid="coding-topology-graph" className="min-w-0 overflow-auto rounded-lg border border-[var(--aria-line)] bg-white" aria-label="单元拓扑图">
      <svg role="presentation" width={width} height={height} viewBox={`0 0 ${width} ${height}`} className="block">
        <defs>
          <marker id="coding-topo-arrow-satisfied" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
            <path d="M0,0 L8,4 L0,8 z" fill="var(--aria-topo-edge)" />
          </marker>
          <marker id="coding-topo-arrow-blocking" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
            <path d="M0,0 L8,4 L0,8 z" fill="var(--aria-topo-node-blocked-fg)" />
          </marker>
        </defs>
        {topology.edges.map((edge) => {
          const from = positions.get(edge.fromWorkItemId);
          const to = positions.get(edge.toWorkItemId);
          if (!from || !to) return null;
          const startX = from.x + NODE_WIDTH;
          const startY = from.y + NODE_HEIGHT / 2;
          const endX = to.x;
          const endY = to.y + NODE_HEIGHT / 2;
          const satisfied = edge.state === "satisfied";
          return (
            <path
              key={`${edge.fromWorkItemId}__${edge.toWorkItemId}`}
              data-edge={`${edge.fromWorkItemId}__${edge.toWorkItemId}`}
              data-edge-state={edge.state}
              d={`M ${startX} ${startY} C ${startX + 32} ${startY}, ${endX - 32} ${endY}, ${endX} ${endY}`}
              fill="none"
              stroke={satisfied ? "var(--aria-topo-edge)" : "var(--aria-topo-node-blocked-fg)"}
              strokeWidth={1.5}
              strokeDasharray={satisfied ? undefined : "6 4"}
              markerEnd={satisfied ? "url(#coding-topo-arrow-satisfied)" : "url(#coding-topo-arrow-blocking)"}
            />
          );
        })}
        {topology.nodes.map((node) => {
          const position = positions.get(node.workItemId);
          if (!position) return null;
          const token = topologyTokenName(node.state);
          const selected = node.workItemId === selectedWorkItemId;
          return (
            <g
              key={node.workItemId}
              data-node={node.workItemId}
              data-state={node.state}
              fill={`var(--aria-topo-node-${token}-bg)`}
              stroke={selected ? "var(--aria-topo-edge-active)" : `var(--aria-topo-node-${token}-border)`}
              strokeWidth={selected ? 2 : 1}
              className={[
                node.state === "awaiting_triage" ? "aria-pulse" : undefined,
                "cursor-pointer rounded-md transition-colors duration-200 focus-visible:outline-2 focus-visible:outline-[var(--aria-primary)]",
              ]
                .filter(Boolean)
                .join(" ")}
              role="button"
              tabIndex={0}
              aria-label={`${node.title} · ${CODING_UNIT_STATE_LABELS[node.state]}`}
              onClick={() => onSelectWorkItem(node.workItemId)}
              onKeyDown={(event) => handleKeyDown(event, node.workItemId)}
            >
              <rect x={position.x} y={position.y} width={NODE_WIDTH} height={NODE_HEIGHT} rx={8} />
              <text
                x={position.x + 12}
                y={position.y + 18}
                fontSize={12}
                fontWeight={600}
                fill={`var(--aria-topo-node-${token}-fg)`}
              >
                {node.title.length > 18 ? `${node.title.slice(0, 17)}…` : node.title}
              </text>
              <text
                x={position.x + 12}
                y={position.y + 34}
                fontSize={10}
                className="aria-mono"
                fill="var(--aria-ink-muted)"
              >
                {`${node.workItemId} · ${CODING_UNIT_STATE_LABELS[node.state]}`}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
