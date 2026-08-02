import { useState } from "react";
import type {
  ProviderPermissionMode,
  WorkspaceProviderName,
  WsProviderConfig,
} from "../../api/types";
import {
  getProviderOptions,
  type ProviderOption,
} from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";

interface ProviderConfigPanelProps {
  providers: WsProviderConfig | null;
  editable: boolean;
  onSelectProvider: (
    role: "author" | "reviewer",
    provider: WorkspaceProviderName,
  ) => void;
  reviewerEnabled: boolean;
  onToggleReviewer: (enabled: boolean) => void;
  permissionModes?: {
    author: ProviderPermissionMode;
    reviewer: ProviderPermissionMode;
  };
  onPermissionModeSelect?: (
    role: "author" | "reviewer",
    mode: ProviderPermissionMode,
  ) => void;
  rounds?: number;
  onChangeRounds?: (rounds: number) => void;
}

const PERMISSION_MODE_LABELS: Record<ProviderPermissionMode, string> = {
  auto: "Auto",
  supervised: "Supervised",
};

export function ProviderConfigPanel({
  providers,
  editable,
  onSelectProvider,
  reviewerEnabled,
  onToggleReviewer,
  permissionModes = { author: "auto", reviewer: "auto" },
  onPermissionModeSelect = () => {},
  rounds = 1,
  onChangeRounds,
}: ProviderConfigPanelProps) {
  const [showAdvanced, setShowAdvanced] = useState(false);
  const snapshot = useProviderAvailabilityStore((state) => state.snapshot);
  const providerOptions = getProviderOptions(snapshot);
  const authorProvider = providerValue(providers?.author, "claude_code");
  const reviewerProvider = providerValue(providers?.reviewer, "codex");
  const currentProviders = new Set<WorkspaceProviderName>([
    authorProvider,
    ...(reviewerEnabled ? [reviewerProvider] : []),
  ]);
  const unavailableOptions = providerOptions.filter(
    (option) =>
      option.disabled &&
      (option.visible || currentProviders.has(option.value)),
  );

  return (
    <section className="space-y-3" aria-label="Provider 配置">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
          Provider 配置
        </h2>
        <span className="text-xs text-[var(--aria-ink-muted)]">
          {editable ? "可编辑" : "已锁定"}
        </span>
      </div>

      <div className="space-y-2">
        <label className="flex items-center gap-2 text-sm">
          <span className="w-16 shrink-0 text-[var(--aria-ink-muted)]">
            Author
          </span>
          <select
            aria-label="Author"
            value={authorProvider}
            onChange={(event) => {
              const provider = event.target.value as WorkspaceProviderName;
              onSelectProvider("author", provider);
              if (provider === "pi") {
                onPermissionModeSelect("author", "auto");
              }
            }}
            disabled={!editable}
            className="min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {providerOptionsForValue(providerOptions, authorProvider).map(
              (provider) => (
                <option
                  key={provider.value}
                  value={provider.value}
                  disabled={provider.disabled}
                >
                  {provider.label}
                </option>
              ),
            )}
          </select>
        </label>

        <PermissionModeControl
          role="author"
          provider={authorProvider}
          mode={permissionModes.author}
          editable={editable}
          onSelect={onPermissionModeSelect}
        />

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
          <>
            <label className="flex items-center gap-2 text-sm">
              <span className="w-16 shrink-0 text-[var(--aria-ink-muted)]">
                Reviewer
              </span>
              <select
                aria-label="Reviewer"
                value={reviewerProvider}
                onChange={(event) => {
                  const provider = event.target.value as WorkspaceProviderName;
                  onSelectProvider("reviewer", provider);
                  if (provider === "pi") {
                    onPermissionModeSelect("reviewer", "auto");
                  }
                }}
                disabled={!editable}
                className="min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
              >
                {providerOptionsForValue(providerOptions, reviewerProvider).map(
                  (provider) => (
                    <option
                      key={provider.value}
                      value={provider.value}
                      disabled={provider.disabled}
                    >
                      {provider.label}
                    </option>
                  ),
                )}
              </select>
            </label>
            <PermissionModeControl
              role="reviewer"
              provider={reviewerProvider}
              mode={permissionModes.reviewer}
              editable={editable}
              onSelect={onPermissionModeSelect}
            />
          </>
        ) : editable ? (
          <div className="rounded-md border border-amber-200 bg-amber-50 px-2 py-1.5 text-xs text-amber-700">
            未启用交叉审核可能降低 artifact 质量
          </div>
        ) : null}
      </div>

      {unavailableOptions.length > 0 ? (
        <div className="space-y-1 rounded-md border border-amber-200 bg-amber-50 px-2 py-1.5 text-xs text-amber-800">
          {unavailableOptions.map((provider) => (
            <p key={provider.value}>
              <span className="font-semibold">{provider.label}：</span>
              {provider.reason ? <span>{provider.reason}</span> : null}
              {provider.installHint ? (
                <span className="ml-1">{provider.installHint}</span>
              ) : null}
            </p>
          ))}
        </div>
      ) : null}

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
            <span className="w-20 shrink-0 text-[var(--aria-ink-muted)]">
              审核轮次
            </span>
            <input
              aria-label="审核轮次"
              type="number"
              min={1}
              max={3}
              value={rounds}
              onChange={(event) =>
                onChangeRounds?.(Number.parseInt(event.target.value, 10))
              }
              disabled={!editable}
              className="h-8 w-20 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            />
          </label>
        </div>
      ) : null}
    </section>
  );
}

function PermissionModeControl({
  role,
  provider,
  mode,
  editable,
  onSelect,
}: {
  role: "author" | "reviewer";
  provider: WorkspaceProviderName;
  mode: ProviderPermissionMode;
  editable: boolean;
  onSelect: (role: "author" | "reviewer", mode: ProviderPermissionMode) => void;
}) {
  const modes: ProviderPermissionMode[] =
    provider === "pi" ? ["auto"] : ["auto", "supervised"];
  const label = role === "author" ? "Author" : "Reviewer";

  return (
    <div
      data-testid={`${role}-permission-mode`}
      className="flex min-w-0 flex-wrap items-center gap-1"
    >
      <span className="w-16 shrink-0 text-sm text-[var(--aria-ink-muted)]">
        {label} 权限
      </span>
      {modes.map((permissionMode) => (
        <button
          key={permissionMode}
          type="button"
          disabled={!editable || permissionMode === mode}
          onClick={() => onSelect(role, permissionMode)}
          aria-pressed={permissionMode === mode}
          className={[
            "inline-flex h-7 cursor-pointer items-center rounded-md border px-2 text-[11px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-45",
            permissionMode === mode
              ? "border-[var(--aria-primary)] bg-white text-[var(--aria-primary)]"
              : "border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
          ].join(" ")}
        >
          {PERMISSION_MODE_LABELS[permissionMode]}
        </button>
      ))}
      {provider === "pi" ? (
        <span className="text-xs text-[var(--aria-ink-muted)]">Pi 仅支持 Auto</span>
      ) : null}
    </div>
  );
}

function providerValue(
  value: WorkspaceProviderName | null | undefined,
  fallback: WorkspaceProviderName,
) {
  return value ?? fallback;
}

function providerOptionsForValue(
  options: ProviderOption[],
  current: WorkspaceProviderName,
) {
  return options.filter((option) => option.visible || option.value === current);
}
