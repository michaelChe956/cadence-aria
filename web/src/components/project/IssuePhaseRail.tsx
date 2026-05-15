import { CheckCircle2, Circle, CircleDot } from "lucide-react";

const PHASES = [
  { id: "clarification", label: "Clarification" },
  { id: "development", label: "Development" },
  { id: "acceptance", label: "Acceptance" },
] as const;

export function IssuePhaseRail({ status }: { status: string | null }) {
  const activeIndex = status === "completed" ? 2 : status === "draft" ? 0 : 1;

  return (
    <nav
      aria-label="Issue 阶段"
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
    >
      <div className="mb-3">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">阶段</h2>
        <p className="mt-0.5 text-xs font-medium text-[var(--aria-ink-muted)]">
          Issue lifecycle
        </p>
      </div>
      <ol className="grid gap-2" aria-label="Issue 阶段列表">
        {PHASES.map((phase, index) => {
          const completed = index < activeIndex;
          const active = index === activeIndex;
          const placeholder = phase.id === "acceptance";
          return (
            <li
              key={phase.id}
              aria-current={active ? "step" : undefined}
              className={
                active
                  ? "flex items-center gap-2 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-2 py-2 text-sm font-semibold text-[var(--aria-ink)]"
                  : "flex items-center gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-2 text-sm font-medium text-[var(--aria-ink-muted)]"
              }
            >
              {completed ? (
                <CheckCircle2 className="h-4 w-4 text-emerald-600" />
              ) : active ? (
                <CircleDot className="h-4 w-4 text-[var(--aria-primary)]" />
              ) : (
                <Circle className="h-4 w-4 text-[var(--aria-ink-muted)]" />
              )}
              <span>{phase.label}</span>
              {placeholder && !active && !completed ? (
                <span className="ml-auto text-[11px] font-medium text-[var(--aria-ink-muted)]">
                  pending
                </span>
              ) : null}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
