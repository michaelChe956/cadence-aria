import { Lock } from "lucide-react";
import type {
  CodingProviderRole,
  CodingRoleProviderConfigSnapshot,
  WorkspaceProviderName,
} from "../../api/types";

const PROVIDERS: WorkspaceProviderName[] = ["fake", "codex", "claude_code"];

const ROLES: Array<{ role: CodingProviderRole; label: string }> = [
  { role: "coder", label: "Coder" },
  { role: "tester", label: "Tester" },
  { role: "analyst", label: "Analyst" },
  { role: "code_reviewer", label: "Code Reviewer" },
  { role: "internal_reviewer", label: "Internal Reviewer" },
];

const PROVIDER_LABELS: Record<WorkspaceProviderName, string> = {
  fake: "Fake",
  codex: "Codex",
  claude_code: "Claude Code",
};

export function CodingProviderConfigPanel({
  snapshot,
  lockedRole,
  onSelect,
}: {
  snapshot: CodingRoleProviderConfigSnapshot | null;
  lockedRole: CodingProviderRole | null;
  onSelect: (role: CodingProviderRole, provider: WorkspaceProviderName) => void;
}) {
  if (!snapshot) {
    return null;
  }

  return (
    <div
      data-testid="coding-provider-config-panel"
      className="border-b border-[var(--aria-line)] bg-white px-3 py-2"
    >
      <div className="grid gap-2 lg:grid-cols-5">
        {ROLES.map(({ role, label }) => {
          const current = snapshot[role];
          const locked = lockedRole === role;
          return (
            <div
              key={role}
              className="min-w-0 rounded-md border border-[var(--aria-line)] px-2 py-2"
            >
              <div className="flex min-w-0 items-center justify-between gap-2">
                <span className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                  {label}
                </span>
                {locked ? (
                  <Lock aria-label={`${label} 已锁定`} className="h-3.5 w-3.5 shrink-0" />
                ) : null}
              </div>
              <div className="mt-1 truncate font-mono text-xs text-[var(--aria-ink-muted)]">
                {current}
              </div>
              <div className="mt-2 flex min-w-0 flex-wrap gap-1">
                {PROVIDERS.map((provider) => (
                  <button
                    key={provider}
                    type="button"
                    disabled={locked || provider === current}
                    onClick={() => onSelect(role, provider)}
                    aria-label={`将 ${label} 切换为 ${PROVIDER_LABELS[provider]}`}
                    className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-45"
                  >
                    {PROVIDER_LABELS[provider]}
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
