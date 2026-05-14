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

function text(value: unknown, fallback = "unknown") {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

function numberText(value: unknown, fallback = "0") {
  return typeof value === "number" || typeof value === "string" ? String(value) : fallback;
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
  const status = text(context.overview.status, "idle");
  const provider = text(context.overview.provider_type ?? context.overview.provider, "internal");
  const attempt = numberText(context.overview.attempt, "1");
  const artifacts = numberText(context.overview.artifact_count, String(context.outputs.length));

  return (
    <section aria-label="Node workspace details" className="min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-black text-[#241B2F]">当前节点</h2>
          <p className="mt-1 text-sm font-semibold text-[#5E516B]">
            当前选中节点的摘要与详细上下文。
          </p>
        </div>
        <span className="rounded-lg border-2 border-indigo-200 bg-indigo-50 px-3 py-1 font-mono text-sm font-bold text-indigo-700">
          {context.node_id ?? "no node"}
        </span>
      </div>
      <div
        role="group"
        aria-label="当前节点摘要"
        className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-2 xl:grid-cols-4"
      >
        <SummaryTile label="Node" value={context.node_id ?? "no node"} tone="indigo" />
        <SummaryTile label="Status" value={status} tone="cyan" />
        <SummaryTile label="Provider" value={provider} tone="orange" />
        <SummaryTile label="Artifacts" value={artifacts} tone="emerald" />
      </div>
      <div
        role="tablist"
        aria-label="Node context tabs"
        className="mt-4 flex gap-1 overflow-x-auto border-b-2 border-indigo-100"
      >
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={selectedTab === tab}
            onClick={() => onSelectTab(tab)}
            className="rounded-t-lg border-2 border-b-0 border-transparent px-3 py-2 text-sm font-bold capitalize text-indigo-500 transition-colors hover:bg-indigo-50 hover:text-indigo-800 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-indigo-200 aria-selected:border-indigo-200 aria-selected:bg-indigo-100 aria-selected:text-indigo-900"
          >
            {tab === "diff" ? "Diff" : tab}
          </button>
        ))}
      </div>
      {selectedTab === "overview" ? (
        <OverviewCard value={context.overview} />
      ) : (
        <pre className="mt-3 max-h-80 min-h-40 overflow-auto rounded-lg border-2 border-indigo-100 bg-indigo-50/80 p-3 text-xs leading-5 text-indigo-950 shadow-inner shadow-indigo-200/80">
          {renderJson(tabItems[selectedTab])}
        </pre>
      )}
    </section>
  );
}

function OverviewCard({ value }: { value: Record<string, unknown> }) {
  const entries = Object.entries(value);
  if (entries.length === 0) {
    return (
      <div className="mt-3 rounded-lg border-2 border-dashed border-indigo-100 bg-indigo-50/70 p-4 text-sm font-semibold text-indigo-600">
        暂无节点概览
      </div>
    );
  }

  return (
    <div className="mt-3 grid gap-2 rounded-lg border-2 border-indigo-100 bg-white p-3 shadow-inner shadow-indigo-100/80">
      {entries.map(([key, item]) => (
        <div
          key={key}
          className="grid grid-cols-[7rem_minmax(0,1fr)] gap-2 rounded-lg bg-indigo-50/80 px-3 py-2 text-sm"
        >
          <span className="font-black capitalize text-indigo-600">{key.replaceAll("_", " ")}</span>
          <span className="break-words font-semibold text-indigo-950">{String(item)}</span>
        </div>
      ))}
    </div>
  );
}

function SummaryTile({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "indigo" | "cyan" | "orange" | "emerald";
}) {
  const toneClass = {
    indigo: "border-indigo-200 bg-indigo-50 text-indigo-800",
    cyan: "border-cyan-200 bg-cyan-50 text-cyan-800",
    orange: "border-orange-200 bg-orange-50 text-orange-800",
    emerald: "border-emerald-200 bg-emerald-50 text-emerald-800",
  }[tone];
  return (
    <div className={`min-w-0 rounded-lg border-2 px-3 py-2 ${toneClass}`}>
      <div className="text-[10px] font-bold uppercase opacity-75">{label}</div>
      <div className="mt-1 truncate font-mono text-sm font-bold">{value}</div>
    </div>
  );
}
