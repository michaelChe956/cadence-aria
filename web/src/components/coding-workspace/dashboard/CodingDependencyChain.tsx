// web/src/components/coding-workspace/dashboard/CodingDependencyChain.tsx
import type { ReactNode } from "react";
import { AlertTriangle, ArrowDownLeft, ArrowUpRight } from "lucide-react";
import type { CodingDependencyChain as CodingDependencyChainData, CodingTopologyNode } from "../../../state/coding-dashboard-projection";
import { CODING_UNIT_STATE_LABELS, topologyTokenName } from "../../../state/coding-dashboard-projection";

function ChainRow({ node, onSelectWorkItem }: { node: CodingTopologyNode; onSelectWorkItem: (id: string) => void }) {
  const token = topologyTokenName(node.state);
  return (
    <button
      type="button"
      onClick={() => onSelectWorkItem(node.workItemId)}
      className="flex min-h-11 w-full items-center gap-2 rounded-md px-2 text-left text-xs transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] focus-visible:outline-2 focus-visible:outline-[var(--aria-primary)]"
    >
      <span
        aria-hidden="true"
        className="h-2 w-2 shrink-0 rounded-sm"
        style={{ background: `var(--aria-topo-node-${token}-fg)` }}
      />
      <span className="min-w-0 flex-1 truncate font-semibold text-[var(--aria-ink)]">{node.title}</span>
      <span className="shrink-0 text-[10px] text-[var(--aria-ink-muted)]">{CODING_UNIT_STATE_LABELS[node.state]}</span>
      <span className="shrink-0 font-mono text-[10px] text-[var(--aria-ink-muted)]">{node.workItemId}</span>
    </button>
  );
}

function ChainColumn({
  title,
  icon,
  nodes,
  emptyTestId,
  emptyCopy,
  onSelectWorkItem,
}: {
  title: string;
  icon: ReactNode;
  nodes: readonly CodingTopologyNode[];
  emptyTestId: string;
  emptyCopy: string;
  onSelectWorkItem: (id: string) => void;
}) {
  return (
    <div className="min-w-0 rounded-lg border border-[var(--aria-line)] bg-white p-2">
      <div className="flex items-center gap-1.5 px-1 pb-1 text-[11px] font-semibold text-[var(--aria-ink)]">
        {icon}
        {title}
      </div>
      {nodes.length === 0 ? (
        <div data-testid={emptyTestId} className="px-1 py-2 text-[11px] text-[var(--aria-ink-muted)]">
          {emptyCopy}
        </div>
      ) : (
        <ul className="flex flex-col">
          {nodes.map((node) => (
            <li key={node.workItemId}>
              <ChainRow node={node} onSelectWorkItem={onSelectWorkItem} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function CodingDependencyChainView({
  chain,
  onSelectWorkItem,
}: {
  chain: CodingDependencyChainData;
  onSelectWorkItem: (workItemId: string) => void;
}) {
  return (
    <section
      data-testid="coding-dependency-chain"
      aria-label="依赖链视图"
      className="grid min-w-0 gap-2 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
    >
      <div className="flex min-h-6 items-center gap-2 px-1 text-xs font-semibold text-[var(--aria-ink)]">
        依赖链
        <span className="font-mono text-[11px] font-normal text-[var(--aria-ink-muted)]">{chain.workItemId}</span>
      </div>
      {chain.blockingSources.length > 0 ? (
        <div
          data-testid="coding-chain-blocking"
          className="flex min-h-11 items-start gap-2 rounded-md border border-[var(--aria-gate-open-border)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]"
        >
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          <span>
            为什么被挡：上游未完成 {chain.blockingSources.length} 个——
            {chain.blockingSources.map((node) => node.title).join("、")}
          </span>
        </div>
      ) : null}
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        <div data-testid="coding-chain-upstream">
          <ChainColumn
            title="上游提供方"
            icon={<ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" />}
            nodes={chain.upstream}
            emptyTestId="coding-chain-no-upstream"
            emptyCopy="无上游依赖"
            onSelectWorkItem={onSelectWorkItem}
          />
        </div>
        <div data-testid="coding-chain-downstream">
          <ChainColumn
            title="下游消费方"
            icon={<ArrowDownLeft className="h-3.5 w-3.5" aria-hidden="true" />}
            nodes={chain.downstream}
            emptyTestId="coding-chain-no-downstream"
            emptyCopy="无下游消费"
            onSelectWorkItem={onSelectWorkItem}
          />
        </div>
      </div>
    </section>
  );
}
