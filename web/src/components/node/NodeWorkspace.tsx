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
    <section
      aria-label="Node workspace details"
      className="min-w-0 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">当前节点</h2>
          <p className="mt-1 text-xs font-medium text-[var(--aria-ink-muted)]">
            当前选中节点的摘要与详细上下文。
          </p>
        </div>
        <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
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
        className="mt-4 flex gap-1 overflow-x-auto border-b border-[var(--aria-line)]"
      >
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={selectedTab === tab}
            onClick={() => onSelectTab(tab)}
            className="rounded-t-md border border-b-0 border-transparent px-3 py-2 text-sm font-semibold capitalize text-[var(--aria-ink-muted)] transition-colors hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] aria-selected:border-[var(--aria-line)] aria-selected:bg-[var(--aria-panel-muted)] aria-selected:text-[var(--aria-ink)]"
          >
            {tab === "diff" ? "Diff" : tab}
          </button>
        ))}
      </div>
      {selectedTab === "overview" ? (
        <OverviewCard value={context.overview} />
      ) : (
        <pre className="mt-3 max-h-80 min-h-40 overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 text-xs leading-5 text-[var(--aria-ink)]">
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
      <div className="mt-3 rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-4 text-sm font-medium text-[var(--aria-ink-muted)]">
        暂无节点概览
      </div>
    );
  }

  return (
    <div className="mt-3 grid gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3">
      {entries.map(([key, item]) => (
        <div
          key={key}
          className="grid grid-cols-[7rem_minmax(0,1fr)] gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-sm"
        >
          <span className="font-semibold capitalize text-[var(--aria-ink-muted)]">
            {key.replaceAll("_", " ")}
          </span>
          <span className="break-words font-medium text-[var(--aria-ink)]">{String(item)}</span>
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
    indigo: "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink)]",
    cyan: "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-ink)]",
    orange: "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
    emerald: "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  }[tone];
  return (
    <div className={`min-w-0 rounded-md border px-3 py-2 ${toneClass}`}>
      <div className="text-[10px] font-semibold uppercase opacity-75">{label}</div>
      <div className="mt-1 truncate font-mono text-sm font-semibold">{value}</div>
    </div>
  );
}
