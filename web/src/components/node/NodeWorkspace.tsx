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
    <main className="min-w-0 p-4">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">Node Workspace</h1>
        <span className="font-mono text-sm text-slate-500">{context.node_id ?? "no node"}</span>
      </div>
      <div className="mt-4 flex gap-1 border-b border-line">
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            aria-pressed={selectedTab === tab}
            onClick={() => onSelectTab(tab)}
            className="rounded-t-md px-3 py-2 text-sm capitalize aria-pressed:bg-white"
          >
            {tab === "diff" ? "Diff" : tab}
          </button>
        ))}
      </div>
      <pre className="mt-3 max-h-[calc(100vh-17rem)] overflow-auto rounded-md border border-line bg-white p-3 text-xs leading-5">
        {renderJson(tabItems[selectedTab])}
      </pre>
    </main>
  );
}
