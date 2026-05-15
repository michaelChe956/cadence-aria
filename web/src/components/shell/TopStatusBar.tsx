type StatusProjection = {
  workspace_root?: string;
  active_task_id?: string | null;
  overview?: Record<string, unknown>;
  git_summary?: {
    branch?: string | null;
    head?: string | null;
    dirty?: boolean;
  };
  sse_connected?: boolean;
  running_state?: string;
};

function text(value: unknown, fallback = "unknown") {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

export function TopStatusBar({ projection }: { projection: StatusProjection | null }) {
  const overview = projection?.overview ?? {};
  const git = projection?.git_summary ?? {};
  const blocked = overview.status === "blocked_by_gate";
  const sseState = projection?.sse_connected ? "connected" : "offline";
  const runningState = text(projection?.running_state, "idle");

  return (
    <section className="text-[var(--aria-ink)]">
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-6">
          <StatusMetric label="Task" value={text(projection?.active_task_id, "no task")} />
          <StatusMetric label="Change" value={text(overview.change_id)} />
          <StatusMetric label="Node" value={text(overview.current_node)} />
          <StatusMetric label="Worktask" value={text(overview.current_worktask)} />
          <StatusMetric label="Policy" value={text(overview.policy_preset)} />
          <StatusMetric label="Provider" value={text(overview.provider_mode)} />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
            Git{" "}
            <strong className="font-mono font-semibold text-[var(--aria-ink)]">
              {text(git.branch, "detached")}
            </strong>{" "}
            <span className="font-mono text-[var(--aria-ink-muted)]">
              {text(git.head, "no head")}
            </span>
            <span
              className={
                git.dirty
                  ? "ml-2 rounded bg-[var(--aria-warning-soft)] px-1.5 py-0.5 text-[var(--aria-warning)]"
                  : "ml-2 rounded bg-[var(--aria-success-soft)] px-1.5 py-0.5 text-[var(--aria-success)]"
              }
            >
              {git.dirty ? "dirty" : "clean"}
            </span>
          </span>
          <StatusPill
            label={`SSE ${sseState}`}
            tone={projection?.sse_connected ? "good" : "muted"}
            value={sseState}
          />
          <StatusPill
            label={`运行状态 ${runningState}`}
            tone={runningState === "running" ? "good" : blocked ? "warn" : "muted"}
            value={runningState}
          />
        </div>
      </div>
      {blocked ? (
        <div className="mt-3 grid grid-cols-1 gap-2 text-xs sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6">
          <BlockedMetric label="Overall" value={text(overview.e2e_overall)} />
          <BlockedMetric label="Business code" value={text(overview.business_code)} />
          <BlockedMetric label="Unit tests" value={text(overview.unit_tests)} />
          <BlockedMetric label="Coverage gate" value={text(overview.coverage_gate)} />
          <BlockedMetric label="Archive worktask" value={text(overview.archive_worktask)} />
          <BlockedMetric label="Root cause" value={text(overview.root_cause)} />
        </div>
      ) : null}
    </section>
  );
}

function StatusMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2">
      <div className="text-[10px] font-semibold uppercase text-[var(--aria-ink-muted)]">
        {label}
      </div>
      <div className="mt-1 truncate font-mono text-xs font-semibold text-[var(--aria-ink)]">
        {value}
      </div>
    </div>
  );
}

function StatusPill({
  label,
  tone,
  value,
}: {
  label: string;
  tone: "good" | "muted" | "warn";
  value: string;
}) {
  const toneClass =
    tone === "good"
      ? "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]"
      : tone === "warn"
        ? "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]"
        : "border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)]";
  return (
    <span
      aria-label={label}
      className={`rounded-md border px-3 py-2 font-mono text-xs font-semibold ${toneClass}`}
    >
      {value}
    </span>
  );
}

function BlockedMetric({ label, value }: { label: string; value: string }) {
  return (
    <span className="rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 font-semibold text-[var(--aria-warning)]">
      {label}: {value}
    </span>
  );
}
