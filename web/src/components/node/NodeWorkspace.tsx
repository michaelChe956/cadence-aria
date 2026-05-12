import type { WorkbenchTab } from "../../state/workbench-store";

const tabs: WorkbenchTab[] = ["overview", "inputs", "run", "outputs", "diff"];

type SelectedNodeContext = {
  node_id: string | null;
  overview: Record<string, unknown>;
  inputs: unknown[];
  run: unknown[];
  outputs: unknown[];
  diffs: unknown[];
};

function renderJson(value: unknown) {
  return JSON.stringify(value, null, 2);
}

export function NodeWorkspace({
  context,
  selectedTab,
  onSelectTab,
}: {
  context: SelectedNodeContext;
  selectedTab: WorkbenchTab;
  onSelectTab: (tab: WorkbenchTab) => void;
}) {
  const tabItems: Record<WorkbenchTab, unknown> = {
    overview: context.overview,
    inputs: context.inputs,
    run: context.run,
    outputs: context.outputs,
    diff: context.diffs,
  };

  return (
    <section aria-label="Node workspace details" className="min-w-0">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-lg font-semibold text-slate-100">Node context</h2>
        <span className="font-mono text-sm text-cyan-200">{context.node_id ?? "no node"}</span>
      </div>
      <div className="mt-4 flex gap-1 border-b border-white/10">
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            aria-pressed={selectedTab === tab}
            onClick={() => onSelectTab(tab)}
            className="rounded-t-md px-3 py-2 text-sm capitalize text-slate-400 hover:bg-white/5 hover:text-slate-100 aria-pressed:bg-cyan-400/10 aria-pressed:text-cyan-100"
          >
            {tab === "diff" ? "Diff" : tab}
          </button>
        ))}
      </div>
      <pre className="mt-3 max-h-80 overflow-auto rounded-lg border border-white/10 bg-black/35 p-3 text-xs leading-5 text-slate-300">
        {renderJson(tabItems[selectedTab])}
      </pre>
    </section>
  );
}
