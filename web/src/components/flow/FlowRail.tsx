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
  const hasTimeline = timeline.length > 0;
  const nodes: TimelineItem[] = timeline;
  const edgesForMarkers = nodes.slice(0, -1).map((item, index) => ({
    source: String(item.node_id ?? `N${String(index).padStart(2, "0")}`),
    target: String(nodes[index + 1].node_id ?? `N${String(index + 1).padStart(2, "0")}`),
  }));

  return (
    <nav
      aria-label="Workflow map"
      className="relative overflow-hidden rounded-lg border-2 border-indigo-200 bg-gradient-to-br from-white via-indigo-50 to-cyan-50 p-4 shadow-[0_10px_0_rgba(79,70,229,0.10),0_18px_34px_rgba(79,70,229,0.16)]"
    >
      <div className="pointer-events-none absolute -right-10 top-12 h-28 w-28 rounded-full bg-orange-200/45 blur-2xl" />
      <div className="pointer-events-none absolute -left-12 bottom-8 h-28 w-28 rounded-full bg-cyan-200/45 blur-2xl" />
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <div className="text-xs font-bold uppercase text-indigo-600">
            Workflow path
          </div>
          <div className="mt-1 text-sm font-semibold text-indigo-950/70">
            节点上下文
          </div>
        </div>
        <div className="rounded-lg border-2 border-indigo-200 bg-indigo-50 px-3 py-1 font-mono text-xs font-bold text-indigo-700">
          {nodes.length} nodes
        </div>
      </div>
      {hasTimeline ? (
        <>
          <div
            data-testid="workflow-path-rail"
            data-motion="ambient"
            className="relative grid gap-3 pl-7"
          >
            <span
              aria-hidden="true"
              className="aria-path-flow absolute bottom-4 left-[0.55rem] top-4 w-1 rounded-full bg-gradient-to-b from-orange-400 via-teal-300 to-rose-300"
            />
            {nodes.map((item, index) => {
              const nodeId = String(item.node_id ?? `N${String(index).padStart(2, "0")}`);
              const selected = selectedNodeId === nodeId;
              return (
                <div key={nodeId} className="relative">
                  <span
                    aria-hidden="true"
                    className={
                      selected
                        ? "aria-selected-dot absolute -left-[1.95rem] top-4 h-5 w-5 rounded-lg border-2 border-indigo-700 bg-orange-400 shadow-[0_0_0_6px_rgba(249,115,22,0.18)]"
                        : "absolute -left-[1.75rem] top-4 h-4 w-4 rounded-lg border-2 border-indigo-200 bg-white shadow-[0_3px_0_rgba(129,140,248,0.18)]"
                    }
                  />
                  <WorkflowNodeButton
                    item={{ ...item, node_id: nodeId }}
                    selected={selected}
                    onSelectNode={onSelectNode}
                  />
                </div>
              );
            })}
          </div>
          <div className="sr-only">
            {edgesForMarkers.map((edge) => (
              <span
                key={`${edge.source}-${edge.target}`}
                data-testid={`workflow-edge-${edge.source}-${edge.target}`}
              />
            ))}
          </div>
        </>
      ) : (
        <div className="rounded-lg border-2 border-dashed border-indigo-200 bg-indigo-50/70 px-4 py-5">
          <div className="text-sm font-bold text-indigo-950">暂无 workflow 节点</div>
          <div className="mt-1 text-sm font-semibold text-indigo-950/65">
            创建任务后，这里会显示执行节点和状态。
          </div>
        </div>
      )}
    </nav>
  );
}

function WorkflowNodeButton({
  item,
  selected,
  onSelectNode,
}: {
  item: TimelineItem;
  selected: boolean;
  onSelectNode: (nodeId: string) => void;
}) {
  const nodeId = String(item.node_id ?? "unknown");
  const status = String(item.status ?? "idle");
  const dropped = Boolean(item.dropped) || status === "dropped";
  const provider = String(item.provider_type ?? "");
  const accent = colorForStatus(status, dropped, selected);
  return (
    <button
      type="button"
      aria-pressed={selected}
      data-active={selected ? "true" : "false"}
      data-dropped={dropped ? "true" : "false"}
      onClick={() => onSelectNode(nodeId)}
      className={`aria-pop-in w-full rounded-lg border-2 px-3 py-3 text-left transition-colors motion-reduce:transition-none ${accent}`}
    >
      <span className="flex items-center justify-between gap-2">
        <span className="font-mono text-sm font-semibold">{nodeId}</span>
        <span className="rounded-md bg-white/75 px-2 py-0.5 text-[10px] font-bold uppercase text-indigo-900">
          {status}
        </span>
      </span>
      <span
        className={
          dropped
            ? "mt-1 block text-xs font-medium text-indigo-950/45 line-through"
            : "mt-1 block text-xs font-medium text-indigo-950/70"
        }
      >
        {provider || "internal"} attempt {String(item.attempt ?? 1)} rework{" "}
        {String(item.rework_count ?? 0)} artifacts {String(item.artifact_count ?? 0)}
        {item.diagnostic ? ` ${String(item.diagnostic)}` : ""}
      </span>
    </button>
  );
}

function colorForStatus(status: string, dropped: boolean, selected: boolean) {
  if (dropped) {
    return "border-slate-300 bg-slate-100 text-slate-500 opacity-80";
  }
  if (selected) {
    return "aria-selected-glow border-[#8E2D60] bg-[#8E2D60] text-white shadow-[0_8px_0_rgba(142,45,96,0.34)] hover:bg-[#A33A70] focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-300";
  }
  if (status === "completed") {
    return "border-emerald-300 bg-emerald-100 text-emerald-950 shadow-[0_6px_0_rgba(16,185,129,0.20)] hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-emerald-200";
  }
  if (status === "running") {
    return "border-cyan-300 bg-cyan-100 text-cyan-950 shadow-[0_6px_0_rgba(6,182,212,0.22)] hover:bg-cyan-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200";
  }
  if (status.includes("blocked") || status === "failed") {
    return "border-orange-300 bg-orange-100 text-orange-950 shadow-[0_6px_0_rgba(249,115,22,0.22)] hover:bg-orange-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200";
  }
  return "border-indigo-200 bg-white text-indigo-950 shadow-[0_6px_0_rgba(129,140,248,0.22)] hover:bg-indigo-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-indigo-200";
}
