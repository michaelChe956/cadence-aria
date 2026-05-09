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
      <section className="border-t border-line bg-white px-4 py-3 text-sm text-slate-600">
        当前没有等待确认的 provider 节点。
      </section>
    );
  }

  return (
    <section className="border-t border-line bg-[#101418] px-4 py-3 text-white">
      <div className="mb-2 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm font-semibold">
            {pendingStep.node_id} · {pendingStep.provider_type}
          </div>
          <div className="text-xs text-slate-300">scope: {scope}</div>
          <div className="text-xs text-slate-300">
            inputs: {pendingStep.canonical_input_refs.join(", ")}
          </div>
          <div className="text-xs text-slate-300">context: {pendingStep.context_files.join(", ")}</div>
          <div className="text-xs text-slate-300">
            forbidden: {pendingStep.forbidden_actions.join(", ")}
          </div>
          <div className="text-xs text-slate-300">
            verify: {pendingStep.verification_commands.join(", ")}
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            className="rounded-md border border-slate-600 px-3 py-2 text-sm"
            onClick={() => onRollback(pendingStep.checkpoint_id)}
          >
            <RotateCcw className="mr-1 inline h-4 w-4" /> 回退
          </button>
          <button
            type="button"
            className="rounded-md border border-slate-600 px-3 py-2 text-sm disabled:opacity-50"
            disabled={!running}
            onClick={onStop}
          >
            <Square className="mr-1 inline h-4 w-4" /> 停止
          </button>
          <button
            type="button"
            className="rounded-md bg-signal px-3 py-2 text-sm font-semibold text-ink"
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
      <label className="block text-xs font-semibold text-slate-300" htmlFor="provider-prompt">
        Provider prompt
      </label>
      <label className="mt-2 block text-xs font-semibold text-slate-300">
        Policy override
        <select
          aria-label="Policy override"
          className="ml-2 rounded-md border border-slate-700 bg-[#151b20] px-2 py-1"
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
        className="mt-1 min-h-32 w-full rounded-md border border-slate-700 bg-[#151b20] p-3 font-mono text-sm text-white outline-none focus:border-signal"
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
      />
    </section>
  );
}
