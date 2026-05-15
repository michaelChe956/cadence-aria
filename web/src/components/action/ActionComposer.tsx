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
    provider_type?: string | null;
  }) => void;
  onRollback: (checkpointId: string) => void;
  onStop: () => void;
  running: boolean;
}) {
  const [prompt, setPrompt] = useState(pendingStep?.prompt ?? "");
  const [policyOverride, setPolicyOverride] = useState<string>("");
  const [providerType, setProviderType] = useState<"claude_code" | "codex">("codex");
  const scope = useMemo(() => pendingStep?.allowed_write_scope.join(", ") ?? "", [pendingStep]);

  useEffect(() => {
    setPrompt(pendingStep?.prompt ?? "");
    setPolicyOverride("");
    setProviderType(normalizeProvider(pendingStep?.provider_type));
  }, [pendingStep]);

  if (!pendingStep) {
    return (
      <section className="rounded-lg border border-dashed border-[var(--aria-line-strong)] bg-[var(--aria-panel-muted)] px-4 py-3 text-sm font-medium text-[var(--aria-ink-muted)]">
        当前没有等待确认的 provider 节点。
      </section>
    );
  }

  return (
    <section className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-3 text-[var(--aria-ink)] shadow-sm">
      <div className="mb-3 flex flex-col justify-between gap-4 xl:flex-row xl:items-start">
        <div className="min-w-0">
          <div className="font-mono text-sm font-semibold text-[var(--aria-ink)]">
            {pendingStep.node_id} · {providerType}
          </div>
          <div className="mt-2 grid gap-1.5 text-xs font-medium md:grid-cols-2">
            <MetaLine label="input summary" value={formatSummary(pendingStep.input_summary)} />
            <MetaLine label="input refs" value={pendingStep.canonical_input_refs.join(", ")} />
            <MetaLine label="allowed write scope" value={scope} />
            <MetaLine
              label="verification commands"
              value={pendingStep.verification_commands.join(", ")}
            />
            <MetaLine label="context files" value={pendingStep.context_files.join(", ")} />
            <MetaLine label="forbidden actions" value={pendingStep.forbidden_actions.join(", ")} />
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          <button
            type="button"
            className="inline-flex h-9 items-center rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)]"
            onClick={() => onRollback(pendingStep.checkpoint_id)}
          >
            <RotateCcw className="mr-1 inline h-4 w-4" /> 回退
          </button>
          <button
            type="button"
            className="inline-flex h-9 items-center rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            disabled={!running}
            onClick={onStop}
          >
            <Square className="mr-1 inline h-4 w-4" /> 停止
          </button>
          <button
            type="button"
            className="inline-flex h-9 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            onClick={() =>
              onConfirm({
                checkpoint_id: pendingStep.checkpoint_id,
                prompt,
                policy_override: policyOverride || null,
                provider_type: providerType,
              })
            }
          >
            <Play className="mr-1 inline h-4 w-4" /> 确认执行
          </button>
        </div>
      </div>
      <div className="rounded-lg border border-dashed border-[var(--aria-line-strong)] bg-[var(--aria-panel-muted)] p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <label className="text-xs font-semibold text-[var(--aria-ink)]" htmlFor="provider-prompt">
            Provider prompt
          </label>
          <div className="flex flex-wrap gap-2">
            <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              Provider
              <select
                aria-label="Provider"
                className="ml-2 h-8 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-2 text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                value={providerType}
                onChange={(event) => setProviderType(normalizeProvider(event.target.value))}
              >
                <option value="claude_code">Claude Code</option>
                <option value="codex">Codex</option>
              </select>
            </label>
            <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              Policy override
              <select
                aria-label="Policy override"
                className="ml-2 h-8 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-2 text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
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
          </div>
        </div>
        <textarea
          id="provider-prompt"
          className="mt-2 min-h-28 w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 font-mono text-sm leading-6 text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
        />
      </div>
    </section>
  );
}

function normalizeProvider(value: string | undefined | null): "claude_code" | "codex" {
  return value === "claude_code" ? "claude_code" : "codex";
}

function MetaLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1">
      <span className="text-[var(--aria-ink-muted)]">{label}</span>
      <span className="text-[var(--aria-ink-muted)]">: </span>
      <span className="break-words font-mono text-[var(--aria-ink)]">{value || "none"}</span>
    </div>
  );
}

function formatSummary(value: unknown) {
  if (value === null || value === undefined) {
    return "";
  }
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(omitSensitiveInputFields(value));
  } catch {
    return String(value);
  }
}

function omitSensitiveInputFields(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(omitSensitiveInputFields);
  }
  if (typeof value === "object" && value !== null) {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([key]) => key !== "prompt" && key !== "input_full")
        .map(([key, entryValue]) => [key, omitSensitiveInputFields(entryValue)]),
    );
  }
  return value;
}
