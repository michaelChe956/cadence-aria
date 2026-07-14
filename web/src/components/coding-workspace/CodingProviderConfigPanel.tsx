import { Lock } from "lucide-react";
import type {
  CodingAttemptScope,
  CodingProviderPermissionMode,
  CodingProviderSelectRole,
  CodingProviderRole,
  CodingRoleProviderConfigSnapshot,
  WorkspaceProviderName,
} from "../../api/types";
import {
  getProviderOptions,
  type ProviderOption,
} from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";

type ProviderConfigRow = {
  selectRole: CodingProviderSelectRole;
  providerKey: "coder" | "code_reviewer" | "internal_reviewer";
  modeRole?: CodingProviderRole;
  lockRole: CodingProviderRole;
  label: string;
};

const BASE_ROLES: ProviderConfigRow[] = [
  {
    selectRole: "coder",
    providerKey: "coder",
    modeRole: "coder",
    lockRole: "coder",
    label: "Coder",
  },
  {
    selectRole: "code_reviewer",
    providerKey: "code_reviewer",
    modeRole: "code_reviewer",
    lockRole: "code_reviewer",
    label: "Code Reviewer",
  },
];

const GROUP_FINAL_REVIEW_ROLE: ProviderConfigRow = {
  selectRole: "internal_reviewer",
  providerKey: "internal_reviewer",
  modeRole: "internal_reviewer",
  lockRole: "internal_reviewer",
  label: "GroupFinalReview",
};

const PERMISSION_MODE_LABELS: Record<CodingProviderPermissionMode, string> = {
  auto: "Auto",
  supervised: "Supervised",
};

export function CodingProviderConfigPanel({
  snapshot,
  attemptScope,
  lockedRole,
  configLocked,
  maxAutoRework,
  onSelect,
  onPermissionModeSelect,
  onMaxAutoReworkSelect,
}: {
  snapshot: CodingRoleProviderConfigSnapshot | null;
  attemptScope: CodingAttemptScope | null;
  lockedRole: CodingProviderRole | null;
  configLocked: boolean;
  maxAutoRework: number;
  onSelect: (
    role: CodingProviderSelectRole,
    provider: WorkspaceProviderName,
  ) => void;
  onPermissionModeSelect: (
    role: CodingProviderRole,
    permissionMode: CodingProviderPermissionMode,
  ) => void;
  onMaxAutoReworkSelect: (maxAutoRework: number) => void;
}) {
  const availabilitySnapshot = useProviderAvailabilityStore(
    (state) => state.snapshot,
  );
  if (!snapshot) {
    return null;
  }
  const providerOptions = getProviderOptions(availabilitySnapshot);
  const roles =
    attemptScope === "work_item_group"
      ? [...BASE_ROLES, GROUP_FINAL_REVIEW_ROLE]
      : BASE_ROLES;

  return (
    <div
      data-testid="coding-provider-config-panel"
      className="grid min-w-0 gap-2 bg-white"
    >
      {roles.map(({ selectRole, providerKey, modeRole, lockRole, label }) => {
        const current = snapshot[providerKey];
        const permissionMode = modeRole
          ? snapshot.permission_modes[modeRole]
          : null;
        const locked = configLocked || lockedRole === lockRole;
        const options = providerOptionsForValue(providerOptions, current);
        const unavailableOptions = options.filter((option) => option.disabled);
        return (
          <div
            key={selectRole}
            role="group"
            aria-label={`${label} Provider 配置`}
            className="grid min-w-0 gap-2 rounded-md border border-[var(--aria-line)] px-3 py-2.5 md:grid-cols-[9rem_minmax(0,1fr)]"
          >
            <div className="min-w-0">
              <div className="flex min-w-0 items-center gap-2">
                <span className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                  {label}
                </span>
                {locked ? (
                  <Lock
                    aria-label={`${label} 已锁定`}
                    className="h-3.5 w-3.5 shrink-0"
                  />
                ) : null}
              </div>
              <div className="mt-1 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                {current}
              </div>
            </div>
            <div className="grid min-w-0 gap-2">
              <div className="flex min-w-0 flex-wrap gap-1">
                {options.map((provider) => (
                  <button
                    key={provider.value}
                    type="button"
                    disabled={
                      locked || provider.disabled || provider.value === current
                    }
                    onClick={() => onSelect(selectRole, provider.value)}
                    aria-label={`将 ${label} 切换为 ${provider.label}`}
                    aria-pressed={provider.value === current}
                    title={
                      [provider.reason, provider.installHint]
                        .filter(Boolean)
                        .join("；") || undefined
                    }
                    className={[
                      "inline-flex h-7 cursor-pointer items-center rounded-md border px-2 text-[11px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-45",
                      provider.value === current
                        ? "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                        : "border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
                    ].join(" ")}
                  >
                    {provider.label}
                  </button>
                ))}
              </div>
              {unavailableOptions.length > 0 ? (
                <div className="space-y-1 text-[11px] text-amber-700">
                  {unavailableOptions.map((provider) => (
                    <p key={provider.value}>
                      <span>{provider.reason}</span>
                      {provider.installHint ? (
                        <span className="ml-1">{provider.installHint}</span>
                      ) : null}
                    </p>
                  ))}
                </div>
              ) : null}
              {modeRole ? (
                <div className="flex min-w-0 flex-wrap gap-1">
                  {(["auto", "supervised"] as const).map((mode) => (
                    <button
                      key={mode}
                      type="button"
                      disabled={locked || mode === permissionMode}
                      onClick={() => onPermissionModeSelect(modeRole, mode)}
                      aria-label={`将 ${label} 授权模式切换为 ${PERMISSION_MODE_LABELS[mode]}`}
                      aria-pressed={mode === permissionMode}
                      className={[
                        "inline-flex h-7 cursor-pointer items-center rounded-md border px-2 text-[11px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-45",
                        mode === permissionMode
                          ? "border-[var(--aria-primary)] bg-white text-[var(--aria-primary)]"
                          : "border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
                      ].join(" ")}
                    >
                      {PERMISSION_MODE_LABELS[mode]}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          </div>
        );
      })}
      <div className="grid min-w-0 gap-2 rounded-md border border-[var(--aria-line)] px-3 py-2.5 md:grid-cols-[9rem_minmax(0,1fr)]">
        <div className="min-w-0">
          <div className="truncate text-xs font-semibold text-[var(--aria-ink)]">
            自动修复次数
          </div>
          <div className="mt-1 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
            {maxAutoRework}
          </div>
        </div>
        <input
          aria-label="CodeReview 自动修复次数"
          type="number"
          min={0}
          max={5}
          step={1}
          value={maxAutoRework}
          disabled={configLocked}
          onChange={(event) => {
            const next = Number(event.currentTarget.value);
            if (Number.isFinite(next)) {
              onMaxAutoReworkSelect(next);
            }
          }}
          className="h-8 w-24 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm font-semibold text-[var(--aria-ink)] disabled:cursor-not-allowed disabled:opacity-45"
        />
      </div>
    </div>
  );
}

function providerOptionsForValue(
  options: ProviderOption[],
  current: WorkspaceProviderName,
) {
  return options.filter((option) => option.visible || option.value === current);
}
