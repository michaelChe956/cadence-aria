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

  return (
    <section className="border-b border-cyan-400/10 bg-[#0b1220] px-4 py-3 text-slate-300">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm">
        <strong className="text-cyan-100">Aria Web</strong>
        <span>{text(projection?.active_task_id, "no task")}</span>
        <span>{text(overview.change_id)}</span>
        <span>{text(overview.current_node)}</span>
        <span>{text(overview.current_worktask)}</span>
        <span>{text(overview.policy_preset)}</span>
        <span>{text(overview.provider_mode)}</span>
        <span>
          Git: {text(git.branch, "detached")} {text(git.head, "no head")}
          {git.dirty ? " dirty" : " clean"}
        </span>
        <span>SSE: {projection?.sse_connected ? "connected" : "offline"}</span>
        <span>{text(projection?.running_state, "idle")}</span>
      </div>
      {blocked ? (
        <div className="mt-2 grid grid-cols-2 gap-2 text-xs md:grid-cols-3 xl:grid-cols-6">
          <span>Overall: {text(overview.e2e_overall)}</span>
          <span>Business code: {text(overview.business_code)}</span>
          <span>Unit tests: {text(overview.unit_tests)}</span>
          <span>Coverage gate: {text(overview.coverage_gate)}</span>
          <span>Archive worktask: {text(overview.archive_worktask)}</span>
          <span>Root cause: {text(overview.root_cause)}</span>
        </div>
      ) : null}
    </section>
  );
}
