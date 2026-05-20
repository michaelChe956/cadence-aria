import { useState } from "react";
import type { WorkspaceProviderName, WsProviderConfig } from "../../api/types";

interface ProviderConfigPanelProps {
  providers: WsProviderConfig | null;
  editable: boolean;
  onSelectProvider: (role: "author" | "reviewer", provider: WorkspaceProviderName) => void;
  reviewerEnabled: boolean;
  onToggleReviewer: (enabled: boolean) => void;
  rounds?: number;
  onChangeRounds?: (rounds: number) => void;
}

const PROVIDER_OPTIONS: Array<{ value: WorkspaceProviderName; label: string }> = [
  { value: "claude_code", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "fake", label: "Fake" },
];

export function ProviderConfigPanel({
  providers,
  editable,
  onSelectProvider,
  reviewerEnabled,
  onToggleReviewer,
  rounds = 1,
  onChangeRounds,
}: ProviderConfigPanelProps) {
  const [showAdvanced, setShowAdvanced] = useState(false);

  return (
    <section className="space-y-3" aria-label="Provider 配置">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Provider 配置</h2>
        <span className="text-xs text-[var(--aria-ink-muted)]">
          {editable ? "可编辑" : "已锁定"}
        </span>
      </div>

      <div className="space-y-2">
        <label className="flex items-center gap-2 text-sm">
          <span className="w-16 shrink-0 text-[var(--aria-ink-muted)]">Author</span>
          <select
            aria-label="Author"
            value={providerValue(providers?.author, "claude_code")}
            onChange={(event) =>
              onSelectProvider("author", event.target.value as WorkspaceProviderName)
            }
            disabled={!editable}
            className="min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {PROVIDER_OPTIONS.map((provider) => (
              <option key={provider.value} value={provider.value}>
                {provider.label}
              </option>
            ))}
          </select>
        </label>

        <label className="flex items-center gap-2 text-sm text-[var(--aria-ink)]">
          <input
            type="checkbox"
            checked={reviewerEnabled}
            onChange={(event) => onToggleReviewer(event.target.checked)}
            disabled={!editable}
            className="h-4 w-4 rounded border-[var(--aria-line)]"
          />
          启用交叉审核
        </label>

        {reviewerEnabled ? (
          <label className="flex items-center gap-2 text-sm">
            <span className="w-16 shrink-0 text-[var(--aria-ink-muted)]">Reviewer</span>
            <select
              aria-label="Reviewer"
              value={providerValue(providers?.reviewer, "codex")}
              onChange={(event) =>
                onSelectProvider("reviewer", event.target.value as WorkspaceProviderName)
              }
              disabled={!editable}
              className="min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              {PROVIDER_OPTIONS.map((provider) => (
                <option key={provider.value} value={provider.value}>
                  {provider.label}
                </option>
              ))}
            </select>
          </label>
        ) : editable ? (
          <div className="rounded-md border border-amber-200 bg-amber-50 px-2 py-1.5 text-xs text-amber-700">
            未启用交叉审核可能降低 artifact 质量
          </div>
        ) : null}
      </div>

      <button
        type="button"
        onClick={() => setShowAdvanced((value) => !value)}
        className="text-xs font-medium text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
      >
        高级配置
      </button>

      {showAdvanced ? (
        <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
          <label className="flex items-center gap-2 text-sm">
            <span className="w-20 shrink-0 text-[var(--aria-ink-muted)]">审核轮次</span>
            <input
              aria-label="审核轮次"
              type="number"
              min={1}
              max={3}
              value={rounds}
              onChange={(event) => onChangeRounds?.(Number.parseInt(event.target.value, 10))}
              disabled={!editable}
              className="h-8 w-20 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            />
          </label>
        </div>
      ) : null}
    </section>
  );
}

function providerValue(
  value: WorkspaceProviderName | null | undefined,
  fallback: WorkspaceProviderName,
) {
  return value ?? fallback;
}
