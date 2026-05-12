import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
  type NodeTypes,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useMemo } from "react";

type TimelineItem = Record<string, unknown>;

type WorkflowNodeData = {
  item: TimelineItem;
  selected: boolean;
  onSelectNode: (nodeId: string) => void;
};

type WorkflowNode = Node<WorkflowNodeData, "workflow">;

const nodeTypes: NodeTypes = {
  workflow: WorkflowNodeCard,
};

export function FlowRail({
  timeline,
  selectedNodeId,
  onSelectNode,
}: {
  timeline: TimelineItem[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
}) {
  const nodes: TimelineItem[] =
    timeline.length > 0
      ? timeline
      : Array.from({ length: 29 }, (_, index) => ({
          node_id: `N${String(index).padStart(2, "0")}`,
          status: "idle",
        }));
  const edgesForMarkers = nodes.slice(0, -1).map((item, index) => ({
    source: String(item.node_id ?? `N${String(index).padStart(2, "0")}`),
    target: String(nodes[index + 1].node_id ?? `N${String(index + 1).padStart(2, "0")}`),
  }));
  const { flowNodes, flowEdges } = useMemo(
    () => buildFlowElements(nodes, selectedNodeId, onSelectNode),
    [nodes, onSelectNode, selectedNodeId],
  );

  return (
    <nav aria-label="Workflow map" className="border-b border-cyan-400/15 bg-[#081018] px-4 py-3">
      <div className="mb-2 flex items-center justify-between">
        <div>
          <div className="text-xs font-semibold uppercase tracking-[0.18em] text-cyan-200/80">
            Workflow map
          </div>
          <div className="text-xs text-slate-500">点击节点查看 workspace 中的输入、执行和产物。</div>
        </div>
        <div className="rounded-full border border-cyan-300/20 bg-cyan-300/10 px-3 py-1 text-xs text-cyan-100">
          {nodes.length} nodes
        </div>
      </div>
      <div className="h-44 overflow-hidden rounded-lg border border-cyan-300/15 bg-[#0b1220] shadow-[0_0_40px_rgba(20,184,166,0.10)]">
        <ReactFlow
          colorMode="dark"
          nodes={flowNodes}
          edges={flowEdges}
          nodeTypes={nodeTypes}
          fitView
          maxZoom={1.4}
          minZoom={0.35}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable={false}
          panOnDrag
          zoomOnScroll
          proOptions={{ hideAttribution: true }}
        >
          <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="#1f3b4a" />
          <Controls showInteractive={false} position="bottom-right" />
        </ReactFlow>
      </div>
      <div className="sr-only">
        {nodes.map((item) => {
          const nodeId = String(item.node_id ?? "unknown");
          const dropped = Boolean(item.dropped) || item.status === "dropped";
          return (
            <button
              key={`a11y-${nodeId}`}
              type="button"
              data-dropped={dropped ? "true" : "false"}
              aria-pressed={selectedNodeId === nodeId}
              onClick={() => onSelectNode(nodeId)}
            >
              {nodeId} {String(item.status ?? "idle")} {String(item.provider_type ?? "")} attempt{" "}
              {String(item.attempt ?? 1)} rework {String(item.rework_count ?? 0)} artifacts{" "}
              {String(item.artifact_count ?? 0)}
              {item.diagnostic ? ` ${String(item.diagnostic)}` : ""}
            </button>
          );
        })}
        {edgesForMarkers.map((edge) => (
          <span
            key={`${edge.source}-${edge.target}`}
            data-testid={`workflow-edge-${edge.source}-${edge.target}`}
          />
        ))}
      </div>
    </nav>
  );
}

function buildFlowElements(
  nodes: TimelineItem[],
  selectedNodeId: string | null,
  onSelectNode: (nodeId: string) => void,
): { flowNodes: WorkflowNode[]; flowEdges: Edge[] } {
  const flowNodes: WorkflowNode[] = nodes.map((item, index) => {
    const nodeId = String(item.node_id ?? `N${String(index).padStart(2, "0")}`);
    return {
      id: nodeId,
      type: "workflow",
      position: { x: index * 170, y: index % 2 === 0 ? 20 : 88 },
      data: {
        item: { ...item, node_id: nodeId },
        selected: selectedNodeId === nodeId,
        onSelectNode,
      },
      draggable: false,
    };
  });
  const flowEdges: Edge[] = nodes.slice(0, -1).map((item, index) => {
    const source = String(item.node_id ?? `N${String(index).padStart(2, "0")}`);
    const target = String(nodes[index + 1].node_id ?? `N${String(index + 1).padStart(2, "0")}`);
    const active = source === selectedNodeId || target === selectedNodeId;
    return {
      id: `edge-${source}-${target}`,
      source,
      target,
      type: "smoothstep",
      animated: active,
      style: {
        stroke: active ? "#22d3ee" : "#1e3a4a",
        strokeWidth: active ? 2.5 : 1.5,
      },
    };
  });
  return { flowNodes, flowEdges };
}

function WorkflowNodeCard({ data }: NodeProps<WorkflowNode>) {
  const item = data.item;
  const nodeId = String(item.node_id ?? "unknown");
  const status = String(item.status ?? "idle");
  const dropped = Boolean(item.dropped) || status === "dropped";
  const provider = String(item.provider_type ?? "");
  const accent = colorForStatus(status, dropped, data.selected);
  return (
    <button
      type="button"
      aria-pressed={data.selected}
      data-dropped={dropped ? "true" : "false"}
      onClick={() => data.onSelectNode(nodeId)}
      className={`min-w-36 rounded-lg border px-3 py-2 text-left shadow-lg transition hover:-translate-y-0.5 ${accent}`}
    >
      <span className="flex items-center justify-between gap-2">
        <span className="font-mono text-sm font-semibold">{nodeId}</span>
        <span className="rounded-full bg-white/10 px-2 py-0.5 text-[10px] uppercase tracking-wide">
          {status}
        </span>
      </span>
      <span className={dropped ? "mt-1 block text-xs text-slate-500 line-through" : "mt-1 block text-xs text-slate-300"}>
        {provider || "internal"} attempt {String(item.attempt ?? 1)} rework{" "}
        {String(item.rework_count ?? 0)} artifacts {String(item.artifact_count ?? 0)}
        {item.diagnostic ? ` ${String(item.diagnostic)}` : ""}
      </span>
    </button>
  );
}

function colorForStatus(status: string, dropped: boolean, selected: boolean) {
  if (dropped) {
    return "border-slate-700 bg-slate-950/80 text-slate-400 opacity-70";
  }
  if (selected) {
    return "border-cyan-300 bg-cyan-400/15 text-cyan-50 shadow-cyan-500/20";
  }
  if (status === "completed") {
    return "border-emerald-300/40 bg-emerald-400/10 text-emerald-50";
  }
  if (status === "running") {
    return "border-cyan-300/40 bg-cyan-400/10 text-cyan-50";
  }
  if (status.includes("blocked") || status === "failed") {
    return "border-amber-300/50 bg-amber-400/10 text-amber-50";
  }
  return "border-slate-700 bg-[#0f172a]/95 text-slate-200";
}
