import { Circle, CircleCheck, CircleDot, History, RotateCcw, XCircle } from "lucide-react";
import type { CodingRoleRun, CodingTimelineNode } from "../../api/types";

interface RoleRunHistoryPanelProps {
  roleRuns: CodingRoleRun[];
  timelineNodes: CodingTimelineNode[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
}

export function RoleRunHistoryPanel({
  roleRuns,
  timelineNodes,
  selectedNodeId,
  onSelectNode,
}: RoleRunHistoryPanelProps) {
  const ordered = [...roleRuns].sort((a, b) =>
    a.started_at === b.started_at ? a.run_no - b.run_no : a.started_at.localeCompare(b.started_at),
  );
  const nodeTitleById = new Map(timelineNodes.map((node) => [node.id, node.title]));

  return (
    <section
      data-testid="coding-role-run-history"
      aria-label="角色运行历史"
      className="border-b border-[var(--aria-line)] bg-white px-3 py-2"
    >
      <div className="mb-2 flex min-w-0 items-center gap-2 text-xs font-semibold text-[var(--aria-ink)]">
        <History className="h-3.5 w-3.5" />
        <span>角色运行历史</span>
      </div>
      {ordered.length === 0 ? (
        <div className="text-xs text-[var(--aria-ink-muted)]">暂无角色运行记录</div>
      ) : (
        <div className="flex min-w-0 gap-2 overflow-x-auto pb-1">
          {ordered.map((run) => {
            const selected = run.node_id !== null && run.node_id === selectedNodeId;
            const title = run.node_id ? nodeTitleById.get(run.node_id) ?? run.node_id : "未绑定节点";
            return (
              <button
                key={run.id}
                type="button"
                disabled={!run.node_id}
                onClick={() => run.node_id && onSelectNode(run.node_id)}
                className={[
                  "grid min-w-[13rem] max-w-[18rem] gap-1 rounded-md border px-2 py-1.5 text-left text-xs",
                  selected
                    ? "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)]"
                    : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] hover:bg-white",
                ].join(" ")}
              >
                <div className="flex min-w-0 items-center justify-between gap-2">
                  <span className="truncate font-semibold text-[var(--aria-ink)]">
                    {roleRunTitle(run)}
                  </span>
                  <span className="inline-flex shrink-0 items-center gap-1 text-[var(--aria-ink-muted)]">
                    {statusIcon(run.status)}
                    {roleRunStatusLabel(run.status)}
                  </span>
                </div>
                <div className="truncate text-[var(--aria-ink-muted)]">{title}</div>
                <div className="truncate font-mono text-[var(--aria-ink-muted)]">
                  {run.trigger}
                </div>
                {run.reason_code ? (
                  <div className="truncate text-[var(--aria-ink-muted)]">{run.reason_code}</div>
                ) : null}
                <EventSummary run={run} />
                <RecentEvents run={run} />
                <RefsSummary run={run} />
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}

function EventSummary({ run }: { run: CodingRoleRun }) {
  const summary = run.event_summary;
  if (!summary || summary.event_count === 0) return null;
  return (
    <div className="grid gap-0.5 text-[10px] text-[var(--aria-ink-muted)]">
      <div className="flex min-w-0 items-center gap-1">
        <span className="font-mono">{summary.event_count} events</span>
        {summary.last_event_title ? (
          <span className="truncate">{summary.last_event_title}</span>
        ) : null}
        {summary.last_event_status ? (
          <span className="shrink-0 font-mono">{summary.last_event_status}</span>
        ) : null}
      </div>
      {summary.terminal_reason ? (
        <div className="truncate">{summary.terminal_reason}</div>
      ) : null}
    </div>
  );
}

function RecentEvents({ run }: { run: CodingRoleRun }) {
  const events = run.recent_events ?? [];
  if (events.length === 0) return null;
  return (
    <div className="grid gap-0.5 border-t border-[var(--aria-line)] pt-1">
      {events.slice(-3).map((event) => (
        <div key={`${run.id}:${event.sequence}`} className="grid min-w-0 gap-0.5">
          <div className="flex min-w-0 items-center gap-1 text-[10px] text-[var(--aria-ink-muted)]">
            <span className="shrink-0 font-mono">#{event.sequence}</span>
            <span className="truncate">{event.title ?? event.event_type}</span>
            {event.status ? <span className="shrink-0 font-mono">{event.status}</span> : null}
          </div>
          {event.detail ? (
            <div className="truncate text-[10px] text-[var(--aria-ink-muted)]">
              {event.detail}
            </div>
          ) : null}
          {event.artifact_ref ? (
            <div className="truncate font-mono text-[10px] text-[var(--aria-ink-muted)]">
              {event.artifact_ref}
            </div>
          ) : null}
        </div>
      ))}
    </div>
  );
}

function RefsSummary({ run }: { run: CodingRoleRun }) {
  const refs = [...run.raw_provider_output_refs, ...run.artifact_refs];
  if (refs.length === 0) return null;
  return (
    <div className="grid gap-0.5">
      {refs.slice(0, 2).map((ref) => (
        <div key={ref} className="truncate font-mono text-[10px] text-[var(--aria-ink-muted)]">
          {ref}
        </div>
      ))}
      {refs.length > 2 ? (
        <div className="text-[10px] text-[var(--aria-ink-muted)]">+{refs.length - 2} refs</div>
      ) : null}
    </div>
  );
}

export function roleRunTitle(run: CodingRoleRun) {
  return `${roleLabel(run.role)} #${run.run_no}`;
}

export function roleRunStatusLabel(status: CodingRoleRun["status"]) {
  const labels: Record<CodingRoleRun["status"], string> = {
    running: "运行中",
    completed: "已完成",
    failed: "失败",
    blocked: "阻塞",
    superseded: "已被替代",
    aborted: "已终止",
  };
  return labels[status];
}

function roleLabel(role: CodingRoleRun["role"]) {
  const labels: Record<CodingRoleRun["role"], string> = {
    coder: "Coder",
    tester: "Tester",
    analyst: "Analyst",
    code_reviewer: "Code Reviewer",
    internal_reviewer: "Internal Reviewer",
  };
  return labels[role];
}

function statusIcon(status: CodingRoleRun["status"]) {
  if (status === "running") return <CircleDot className="h-3 w-3" />;
  if (status === "completed") return <CircleCheck className="h-3 w-3" />;
  if (status === "superseded") return <RotateCcw className="h-3 w-3" />;
  if (status === "failed" || status === "aborted") return <XCircle className="h-3 w-3" />;
  return <Circle className="h-3 w-3" />;
}
