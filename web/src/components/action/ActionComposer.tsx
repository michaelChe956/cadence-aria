import { Play, RotateCcw, Square } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { PendingProviderStep } from "../../api/types";

export function ActionComposer({
  pendingStep,
  onConfirm,
  onRollback,
  onStop,
  running,
}: {
  pendingStep: PendingProviderStep | null;
  onConfirm: (payload: {
    checkpoint_id: string;
    prompt: string;
    policy_override?: string | null;
  }) => void;
  onRollback: (checkpointId: string) => void;
  onStop: () => void;
  running: boolean;
}) {
  const [prompt, setPrompt] = useState(pendingStep?.prompt ?? "");
  const [policyOverride, setPolicyOverride] = useState<string>("");
  const scope = useMemo(() => pendingStep?.allowed_write_scope.join(", ") ?? "", [pendingStep]);

  useEffect(() => {
    setPrompt(pendingStep?.prompt ?? "");
    setPolicyOverride("");
  }, [pendingStep]);

  if (!pendingStep) {
    return (
      <section className="rounded-lg border-2 border-dashed border-indigo-200 bg-indigo-50/80 px-4 py-3 text-sm font-semibold text-indigo-700">
        当前没有等待确认的 provider 节点。
      </section>
    );
  }

  return (
    <section className="rounded-lg border-2 border-cyan-200 bg-cyan-50 px-4 py-3 text-indigo-950 shadow-[0_10px_0_rgba(6,182,212,0.16),0_18px_38px_rgba(14,116,144,0.14)]">
      <div className="mb-3 flex flex-col justify-between gap-4 xl:flex-row xl:items-start">
        <div className="min-w-0">
          <div className="font-mono text-sm font-bold text-cyan-950">
            {pendingStep.node_id} · {pendingStep.provider_type}
          </div>
          <div className="mt-2 grid gap-1.5 text-xs font-semibold text-indigo-700 md:grid-cols-2">
            <MetaLine label="scope" value={scope} />
            <MetaLine label="inputs" value={pendingStep.canonical_input_refs.join(", ")} />
            <MetaLine label="context" value={pendingStep.context_files.join(", ")} />
            <MetaLine label="forbidden" value={pendingStep.forbidden_actions.join(", ")} />
            <MetaLine label="verify" value={pendingStep.verification_commands.join(", ")} />
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          <button
            type="button"
            className="inline-flex items-center rounded-lg border-2 border-orange-300 bg-white px-3 py-2 text-sm font-bold text-orange-800 shadow-[0_4px_0_rgba(251,146,60,0.20)] transition-colors hover:bg-orange-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200"
            onClick={() => onRollback(pendingStep.checkpoint_id)}
          >
            <RotateCcw className="mr-1 inline h-4 w-4" /> 回退
          </button>
          <button
            type="button"
            className="inline-flex items-center rounded-lg border-2 border-rose-300 bg-white px-3 py-2 text-sm font-bold text-rose-800 shadow-[0_4px_0_rgba(251,113,133,0.20)] transition-colors hover:bg-rose-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-rose-200 disabled:border-slate-200 disabled:text-slate-400 disabled:shadow-none"
            disabled={!running}
            onClick={onStop}
          >
            <Square className="mr-1 inline h-4 w-4" /> 停止
          </button>
          <button
            type="button"
            className="inline-flex items-center rounded-lg border-2 border-emerald-600 bg-emerald-500 px-3 py-2 text-sm font-bold text-white shadow-[0_5px_0_rgba(6,95,70,0.38)] transition-colors hover:bg-emerald-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-emerald-200"
            onClick={() =>
              onConfirm({
                checkpoint_id: pendingStep.checkpoint_id,
                prompt,
                policy_override: policyOverride || null,
              })
            }
          >
            <Play className="mr-1 inline h-4 w-4" /> 确认执行
          </button>
        </div>
      </div>
      <label className="block text-xs font-bold text-indigo-800" htmlFor="provider-prompt">
        Provider prompt
      </label>
      <label className="mt-2 block text-xs font-bold text-indigo-800">
        Policy override
        <select
          aria-label="Policy override"
          className="ml-2 rounded-lg border-2 border-cyan-200 bg-white px-2 py-1 text-indigo-950 outline-none transition-colors hover:border-orange-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
          value={policyOverride}
          onChange={(event) => setPolicyOverride(event.target.value)}
        >
          <option value="">inherit</option>
          <option value="manual-all">manual-all</option>
          <option value="manual-write">manual-write</option>
          <option value="auto-review">auto-review</option>
          <option value="non-interactive">non-interactive</option>
        </select>
      </label>
      <textarea
        id="provider-prompt"
        className="mt-1 min-h-32 w-full rounded-lg border-2 border-cyan-200 bg-white p-3 font-mono text-sm text-indigo-950 shadow-inner shadow-cyan-200/70 outline-none transition-colors focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
      />
    </section>
  );
}

function MetaLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <span className="text-indigo-500">{label}: </span>
      <span className="break-words font-mono text-indigo-950">{value || "none"}</span>
    </div>
  );
}
