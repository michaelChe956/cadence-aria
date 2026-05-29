import { Clock, Play, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { CodingGateRequired } from "../../api/types";

const GATE_COUNTDOWN_MS = 5_000;

const ROLE_LABELS = {
  coder: "Coder",
  tester: "Tester",
  analyst: "Analyst",
  code_reviewer: "Code Reviewer",
  internal_reviewer: "Internal Reviewer",
} as const;

export function StageGateEntry({
  gate,
  onConfirmStage,
  onAbort,
}: {
  gate: CodingGateRequired;
  onConfirmStage: (stage: NonNullable<CodingGateRequired["stage"]>) => void;
  onAbort: () => void;
}) {
  const [now, setNow] = useState(() => Date.now());
  const expiresAtMs = useMemo(
    () => (gate.expires_at ? new Date(gate.expires_at).getTime() : null),
    [gate.expires_at],
  );
  const remainingMs = expiresAtMs === null ? 0 : Math.max(0, expiresAtMs - now);
  const remainingSeconds = Math.ceil(remainingMs / 1000);
  const progress = Math.max(0, Math.min(100, (remainingMs / GATE_COUNTDOWN_MS) * 100));
  const roleLabel = gate.role ? ROLE_LABELS[gate.role] : "Stage";
  const provider = gate.role && gate.provider_snapshot ? gate.provider_snapshot[gate.role] : null;

  useEffect(() => {
    if (expiresAtMs === null || remainingMs <= 0) {
      return;
    }
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [expiresAtMs, remainingMs]);

  return (
    <div
      data-testid="coding-stage-gate-entry"
      className="border-t border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-3"
    >
      <div className="flex min-w-0 flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <Clock className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
            <div className="truncate text-sm font-semibold text-[var(--aria-ink)]">
              {gate.title}
            </div>
            <span className="shrink-0 rounded bg-white px-1.5 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
              {remainingSeconds > 0 ? `${remainingSeconds}s` : "已自动确认"}
            </span>
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-xs text-[var(--aria-ink-muted)]">
            <span>{roleLabel}</span>
            {provider ? <span className="font-mono">{provider}</span> : null}
            <span className="truncate">{gate.description}</span>
          </div>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-white">
            <div
              className="h-full bg-[var(--aria-primary)] transition-[width]"
              style={{ width: `${progress}%` }}
            />
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {gate.stage ? (
            <button
              type="button"
              onClick={() => onConfirmStage(gate.stage!)}
              aria-label="Stage Gate 立即开始"
              className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]"
            >
              <Play className="h-3.5 w-3.5" />
              立即开始
            </button>
          ) : null}
          <button
            type="button"
            onClick={onAbort}
            aria-label="Stage Gate 中止"
            className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]"
          >
            <X className="h-3.5 w-3.5" />
            中止
          </button>
        </div>
      </div>
    </div>
  );
}
