import { Lock } from "lucide-react";

interface WorkspaceHeaderProps {
  entityType: string;
  entityId: string;
  version?: number | null;
  author: string;
  reviewer: string | null;
  rounds: number;
  stage: string;
  providerLocked: boolean;
  lockedAt?: string | null;
  superpowers?: boolean;
  openSpec?: boolean;
}

const PROVIDER_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  fake: "Fake",
};

const STAGE_BADGES: Record<string, { text: string; className: string }> = {
  prepare_context: {
    text: "准备中",
    className: "border-[var(--aria-line)] bg-white text-[var(--aria-ink-muted)]",
  },
  running: {
    text: "运行中 · 保持本页打开",
    className: "border-amber-200 bg-amber-50 text-amber-800",
  },
  cross_review: {
    text: "审核中",
    className: "border-sky-200 bg-sky-50 text-sky-800",
  },
  review_decision: {
    text: "审核结论待处理",
    className: "border-violet-200 bg-violet-50 text-violet-800",
  },
  revision: {
    text: "修订中",
    className: "border-orange-200 bg-orange-50 text-orange-800",
  },
  human_confirm: {
    text: "等待确认",
    className: "border-emerald-200 bg-emerald-50 text-emerald-800",
  },
  completed: {
    text: "已完成",
    className: "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
  },
};

export function WorkspaceHeader({
  entityType,
  entityId,
  version,
  author,
  reviewer,
  rounds,
  stage,
  providerLocked,
  lockedAt,
  superpowers = false,
  openSpec = false,
}: WorkspaceHeaderProps) {
  const badge = STAGE_BADGES[stage] ?? STAGE_BADGES.prepare_context;

  return (
    <header className="border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-3">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-2 text-sm text-[var(--aria-ink-muted)]">
            <span className="font-semibold text-[var(--aria-ink)]">
              {entityType} #{entityId}
            </span>
            {version ? <span className="font-mono text-xs">v{version}</span> : null}
            {providerLocked ? (
              <Lock
                aria-label="Provider 已锁定"
                className="h-3.5 w-3.5 text-[var(--aria-primary)]"
                data-locked-at={lockedAt ?? undefined}
              />
            ) : null}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-[var(--aria-ink-muted)]">
            <span>Author: {providerLabel(author)}</span>
            {reviewer ? <span>Reviewer: {providerLabel(reviewer)}</span> : null}
            <span>
              {rounds} round{rounds === 1 ? "" : "s"}
            </span>
            <span>Superpowers: {superpowers ? "on" : "off"}</span>
            <span>OpenSpec: {openSpec ? "on" : "off"}</span>
          </div>
        </div>
        <span
          data-testid="stage-badge"
          className={`shrink-0 rounded-md border px-2 py-1 text-xs font-semibold ${badge.className}`}
        >
          {badge.text}
        </span>
      </div>
    </header>
  );
}

function providerLabel(provider: string) {
  return PROVIDER_LABELS[provider] ?? provider;
}
