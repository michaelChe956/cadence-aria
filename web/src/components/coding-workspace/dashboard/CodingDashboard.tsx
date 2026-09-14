// web/src/components/coding-workspace/dashboard/CodingDashboard.tsx
import { useEffect, useMemo, useState } from "react";
import { Activity, ChevronDown, ChevronUp } from "lucide-react";
import {
  selectCodingBudgetGates,
  selectCodingDependencyChain,
  selectCodingTopology,
} from "../../../state/coding-dashboard-projection";
import { useCodingWorkspaceStore } from "../../../state/coding-workspace-store";
import { CodingBudgetGates } from "./CodingBudgetGates";
import { CodingDependencyChainView } from "./CodingDependencyChain";
import { CodingLogConsole } from "./CodingLogConsole";
import { CodingTopologyGraph } from "./CodingTopologyGraph";

export function CodingDashboard() {
  const units = useCodingWorkspaceStore((state) => state.units);
  const timelineNodes = useCodingWorkspaceStore((state) => state.timelineNodes);
  const currentWorkItemId = useCodingWorkspaceStore((state) => state.currentWorkItemId);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [selectedWorkItemId, setSelectedWorkItemId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  const focusWorkItemId = selectedWorkItemId ?? currentWorkItemId;
  const topology = useMemo(() => selectCodingTopology(units), [units]);
  const chain = useMemo(
    () => (focusWorkItemId ? selectCodingDependencyChain(units, focusWorkItemId) : null),
    [focusWorkItemId, units],
  );
  const gates = useMemo(() => selectCodingBudgetGates(timelineNodes, nowMs), [timelineNodes, nowMs]);

  return (
    <section
      data-testid="coding-dashboard"
      aria-label="编码仪表盘"
      className="shrink-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    >
      <div className="flex h-11 items-center justify-between gap-2 px-4">
        <div className="flex items-center gap-2 text-xs font-semibold text-[var(--aria-ink)]">
          <Activity className="h-4 w-4" aria-hidden="true" />
          编码仪表盘
          <span className="text-[10px] font-normal text-[var(--aria-ink-muted)]">只读视图 · 动作入口统一收口于 Phase 4</span>
        </div>
        <button
          type="button"
          aria-label={collapsed ? "展开编码仪表盘" : "折叠编码仪表盘"}
          onClick={() => setCollapsed((value) => !value)}
          className="inline-flex h-11 w-11 items-center justify-center rounded-md text-[var(--aria-ink-muted)] transition-colors duration-200 hover:bg-[var(--aria-panel)] focus-visible:outline-2 focus-visible:outline-[var(--aria-primary)]"
        >
          {collapsed ? <ChevronDown className="h-4 w-4" aria-hidden="true" /> : <ChevronUp className="h-4 w-4" aria-hidden="true" />}
        </button>
      </div>
      {collapsed ? null : (
        <div className="grid max-h-[45vh] grid-cols-1 gap-3 overflow-auto px-4 pb-3 lg:grid-cols-[minmax(0,1fr)_20rem]">
          <div className="grid min-w-0 content-start gap-3">
            <CodingTopologyGraph
              topology={topology}
              selectedWorkItemId={focusWorkItemId}
              onSelectWorkItem={setSelectedWorkItemId}
            />
            {chain ? (
              <CodingDependencyChainView chain={chain} onSelectWorkItem={setSelectedWorkItemId} />
            ) : null}
          </div>
          <div className="grid min-w-0 grid-rows-[auto_minmax(0,1fr)] content-start gap-3">
            <CodingBudgetGates gates={gates} nowMs={nowMs} />
            <CodingLogConsole />
          </div>
        </div>
      )}
    </section>
  );
}
