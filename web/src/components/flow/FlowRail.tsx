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
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <div className="text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
            Workflow path
          </div>
          <div className="mt-1 text-sm font-medium text-[var(--aria-ink)]">
            节点上下文
          </div>
        </div>
        <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
          {nodes.length} nodes
        </div>
      </div>
      {hasTimeline ? (
        <>
          <div
            data-testid="workflow-path-rail"
            data-motion="ambient"
            className="relative grid gap-2 pl-6"
          >
            <span
              aria-hidden="true"
              className="absolute bottom-4 left-[0.45rem] top-4 w-px bg-[var(--aria-line-strong)]"
            />
            {nodes.map((item, index) => {
              const nodeId = String(item.node_id ?? `N${String(index).padStart(2, "0")}`);
              const selected = selectedNodeId === nodeId;
              return (
                <div key={`${nodeId}-${index}`} className="relative">
                  <span
                    aria-hidden="true"
                    className={
                      selected
                        ? "absolute -left-[1.95rem] top-4 h-4 w-4 rounded border border-[var(--aria-primary)] bg-[var(--aria-primary)]"
                        : "absolute -left-[1.8rem] top-4 h-3 w-3 rounded border border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"
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
            {edgesForMarkers.map((edge, index) => (
              <span
                key={`${edge.source}-${edge.target}-${index}`}
                data-testid={`workflow-edge-${edge.source}-${edge.target}`}
              />
            ))}
          </div>
        </>
      ) : (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-5">
          <div className="text-sm font-semibold text-[var(--aria-ink)]">暂无 workflow 节点</div>
          <div className="mt-1 text-sm font-medium text-[var(--aria-ink-muted)]">
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
      className={`w-full rounded-md border px-3 py-2 text-left transition-colors motion-reduce:transition-none ${accent}`}
    >
      <span className="flex items-center justify-between gap-2">
        <span className="font-mono text-sm font-semibold">{nodeId}</span>
        <span className="rounded border border-current/20 bg-[var(--aria-panel)] px-1.5 py-0.5 text-[10px] font-semibold uppercase">
          {status}
        </span>
      </span>
      <span
        className={
          dropped
            ? "mt-1 block text-xs font-medium text-[var(--aria-ink-muted)] line-through"
            : "mt-1 block text-xs font-medium text-[var(--aria-ink-muted)]"
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
    return "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)] opacity-80";
  }
  if (selected) {
    return "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]";
  }
  if (status === "completed") {
    return "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-ink)] hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-success)]";
  }
  if (status === "running") {
    return "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-ink)] hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]";
  }
  if (status.includes("blocked") || status === "failed") {
    return "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-ink)] hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-warning)]";
  }
  return "border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]";
}
