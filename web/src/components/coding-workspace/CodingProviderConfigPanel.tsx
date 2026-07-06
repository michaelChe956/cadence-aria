import { Lock } from "lucide-react";
import type {
  CodingProviderPermissionMode,
  CodingProviderSelectRole,
  CodingProviderRole,
  CodingRoleProviderConfigSnapshot,
  WorkspaceProviderName,
} from "../../api/types";

const PROVIDERS: WorkspaceProviderName[] = ["fake", "codex", "claude_code"];

type ProviderConfigRow = {
  selectRole: CodingProviderSelectRole;
  providerKey:
    | "coder"
    | "tester_plan"
    | "tester_execute"
    | "analyst"
    | "code_reviewer"
    | "internal_reviewer";
  modeRole?: CodingProviderRole;
  lockRole: CodingProviderRole;
  label: string;
};

const ROLES: ProviderConfigRow[] = [
  { selectRole: "coder", providerKey: "coder", modeRole: "coder", lockRole: "coder", label: "Coder" },
  {
    selectRole: "tester_plan",
    providerKey: "tester_plan",
    lockRole: "tester",
    label: "Tester Plan",
  },
  {
    selectRole: "tester_execute",
    providerKey: "tester_execute",
    modeRole: "tester",
    lockRole: "tester",
    label: "Tester Execute",
  },
  {
    selectRole: "analyst",
    providerKey: "analyst",
    modeRole: "analyst",
    lockRole: "analyst",
    label: "Analyst",
  },
  {
    selectRole: "code_reviewer",
    providerKey: "code_reviewer",
    modeRole: "code_reviewer",
    lockRole: "code_reviewer",
    label: "Code Reviewer",
  },
  {
    selectRole: "internal_reviewer",
    providerKey: "internal_reviewer",
    modeRole: "internal_reviewer",
    lockRole: "internal_reviewer",
    label: "Internal Reviewer",
  },
];

const PROVIDER_LABELS: Record<WorkspaceProviderName, string> = {
  fake: "Fake",
  codex: "Codex",
  claude_code: "Claude Code",
};

const PERMISSION_MODE_LABELS: Record<CodingProviderPermissionMode, string> = {
  auto: "Auto",
  supervised: "Supervised",
};

export function CodingProviderConfigPanel({
  snapshot,
  lockedRole,
  onSelect,
  onPermissionModeSelect,
}: {
  snapshot: CodingRoleProviderConfigSnapshot | null;
  lockedRole: CodingProviderRole | null;
  onSelect: (role: CodingProviderSelectRole, provider: WorkspaceProviderName) => void;
  onPermissionModeSelect: (
    role: CodingProviderRole,
    permissionMode: CodingProviderPermissionMode,
  ) => void;
}) {
  if (!snapshot) {
    return null;
  }

  return (
    <div
      data-testid="coding-provider-config-panel"
      className="grid min-w-0 gap-2 bg-white"
    >
      {ROLES.map(({ selectRole, providerKey, modeRole, lockRole, label }) => {
        if (
          selectRole === "tester_plan" ||
          selectRole === "tester_execute" ||
          selectRole === "analyst"
        ) {
          return null;
        }
        const current = snapshot[providerKey];
        const permissionMode = modeRole ? snapshot.permission_modes[modeRole] : null;
        const locked = lockedRole === lockRole;
        return (
          <div
            key={selectRole}
            className="grid min-w-0 gap-2 rounded-md border border-[var(--aria-line)] px-3 py-2.5 md:grid-cols-[9rem_minmax(0,1fr)]"
          >
            <div className="min-w-0">
              <div className="flex min-w-0 items-center gap-2">
                <span className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                  {label}
                </span>
                {locked ? (
                  <Lock aria-label={`${label} 已锁定`} className="h-3.5 w-3.5 shrink-0" />
                ) : null}
              </div>
              <div className="mt-1 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                {current}
              </div>
            </div>
            <div className="grid min-w-0 gap-2">
              <div className="flex min-w-0 flex-wrap gap-1">
                {PROVIDERS.map((provider) => (
                  <button
                    key={provider}
                    type="button"
                    disabled={locked || provider === current}
                    onClick={() => onSelect(selectRole, provider)}
                    aria-label={`将 ${label} 切换为 ${PROVIDER_LABELS[provider]}`}
                    aria-pressed={provider === current}
                    className={[
                      "inline-flex h-7 cursor-pointer items-center rounded-md border px-2 text-[11px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-45",
                      provider === current
                        ? "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                        : "border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
                    ].join(" ")}
                  >
                    {PROVIDER_LABELS[provider]}
                  </button>
                ))}
              </div>
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
    </div>
  );
}
