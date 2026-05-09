type TimelineItem = Record<string, unknown>;

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

  return (
    <nav aria-label="Node flow" className="border-r border-line bg-panel p-3">
      <div className="mb-3 text-xs font-semibold uppercase text-slate-500">Flow</div>
      <div className="space-y-1">
        {nodes.map((item) => {
          const nodeId = String(item.node_id ?? "unknown");
          const dropped = Boolean(item.dropped) || item.status === "dropped";
          return (
            <button
              key={nodeId}
              type="button"
              data-dropped={dropped ? "true" : "false"}
              aria-pressed={selectedNodeId === nodeId}
              onClick={() => onSelectNode(nodeId)}
              className="grid w-full grid-cols-[3.5rem_1fr] items-center gap-2 rounded-md px-2 py-2 text-left text-sm hover:bg-white aria-pressed:bg-white"
            >
              <span className="font-mono font-semibold">{nodeId}</span>
              <span className={dropped ? "text-slate-400 line-through" : "text-slate-700"}>
                {String(item.status ?? "idle")} {String(item.provider_type ?? "")} attempt{" "}
                {String(item.attempt ?? 1)} rework {String(item.rework_count ?? 0)} artifacts{" "}
                {String(item.artifact_count ?? 0)}
                {item.diagnostic ? ` ${String(item.diagnostic)}` : ""}
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
}
