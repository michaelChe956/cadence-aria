import { topologyTokenName, type CockpitFlowState } from "../../../state/workspace-cockpit-projection";

export function CockpitMiniTopology({
  states,
  currentIndex,
}: {
  states: readonly CockpitFlowState[];
  currentIndex: number;
}) {
  return (
    <span
      data-testid="cockpit-mini-topology"
      aria-hidden="true"
      className="inline-flex shrink-0 items-center gap-0.5"
    >
      {states.map((state, index) => (
        <span
          key={`${index}-${state}`}
          className="h-1.5 w-1.5 rounded-sm"
          style={{
            background: `var(--aria-topo-node-${topologyTokenName(state)}-fg)`,
            outline:
              index === currentIndex ? "1px solid var(--aria-topo-edge-active)" : undefined,
          }}
        />
      ))}
    </span>
  );
}
