import { Circle, CircleCheck, CircleDot, History, RotateCcw, XCircle } from "lucide-react";
import { useState } from "react";
import type { CodingRoleRun, CodingTimelineNode } from "../../api/types";
import { elapsedDurationText } from "../shared/duration";

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
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);
  const ordered = orderRoleRuns(roleRuns);
  const nodeTitleById = new Map(timelineNodes.map((node) => [node.id, node.title]));

  return (
    <section
      data-testid="coding-role-run-history"
      aria-label="角色运行历史"
      className="min-w-0 overflow-hidden bg-white"
    >
      <div className="mb-3 flex min-w-0 items-center gap-2 text-xs font-semibold text-[var(--aria-ink)]">
        <History className="h-3.5 w-3.5" />
        <span>角色运行历史</span>
      </div>
      {ordered.length === 0 ? (
        <div className="text-xs text-[var(--aria-ink-muted)]">暂无角色运行记录</div>
      ) : (
        <div className="grid min-w-0 gap-2">
          {ordered.map((run) => {
            const selected = run.node_id !== null && run.node_id === selectedNodeId;
            const expanded = expandedRunId === run.id;
            const title = run.node_id ? nodeTitleById.get(run.node_id) ?? run.node_id : "未绑定节点";
            const duration = elapsedDurationText(run.started_at, run.completed_at);
            return (
              <button
                key={run.id}
                type="button"
                disabled={!run.node_id}
                onClick={() => {
                  setExpandedRunId((current) => (current === run.id ? null : run.id));
                  if (run.node_id) {
                    onSelectNode(run.node_id);
                  }
                }}
                aria-expanded={expanded}
                className={[
                  "grid min-w-0 gap-1 rounded-md border px-2.5 py-2 text-left text-xs transition-colors",
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
                    {duration ? <span className="font-mono">耗时 {duration}</span> : null}
                  </span>
                </div>
                <div className="truncate text-[var(--aria-ink-muted)]">{title}</div>
                <div
                  aria-atomic="true"
                  aria-live="polite"
                  className="grid min-w-0 gap-0.5 text-[var(--aria-ink-muted)]"
                >
                  <div className="truncate font-mono">{retryDescription(run)}</div>
                  {run.retry_metadata ? (
                    <div className="truncate font-mono">
                      周期：{run.retry_metadata.cycle_id} · 第 {run.retry_metadata.attempt_no}/3 次
                    </div>
                  ) : null}
                  {run.reason_code ? (
                    <div className="truncate">
                      {run.status === "failed" ? "失败：" : "原因："}
                      {run.reason_code}
                    </div>
                  ) : null}
                  {isAutomaticRetryExhausted(run) ? (
                    <div>自动重试已耗尽，等待人工处理</div>
                  ) : null}
                </div>
                <EventSummary run={run} />
                {expanded ? (
                  <div className="grid min-w-0 gap-2 border-t border-[var(--aria-line)] pt-2">
                    <EventDetails run={run} />
                    <RecentEvents run={run} />
                    <RefsSummary run={run} />
                  </div>
                ) : null}
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}

function orderRoleRuns(roleRuns: CodingRoleRun[]) {
  const cycleStartedAt = new Map<string, string>();
  for (const run of roleRuns) {
    const cycleId = roleRunCycleId(run);
    const firstStartedAt = cycleStartedAt.get(cycleId);
    if (!firstStartedAt || run.started_at < firstStartedAt) {
      cycleStartedAt.set(cycleId, run.started_at);
    }
  }
  return [...roleRuns].sort((a, b) => {
    const aCycleId = roleRunCycleId(a);
    const bCycleId = roleRunCycleId(b);
    const byCycleStart = (cycleStartedAt.get(aCycleId) ?? "").localeCompare(
      cycleStartedAt.get(bCycleId) ?? "",
    );
    if (byCycleStart !== 0) return byCycleStart;
    const byCycleId = aCycleId.localeCompare(bCycleId);
    if (byCycleId !== 0) return byCycleId;
    const byOrdinal = (a.retry_metadata?.attempt_no ?? a.run_no) - (b.retry_metadata?.attempt_no ?? b.run_no);
    if (byOrdinal !== 0) return byOrdinal;
    return a.id.localeCompare(b.id);
  });
}

function roleRunCycleId(run: CodingRoleRun) {
  return run.retry_metadata?.cycle_id ?? `legacy:${run.id}`;
}

function retryDescription(run: CodingRoleRun) {
  const retry = run.retry_metadata;
  if (run.trigger === "automatic_retry" && retry) {
    return `第 ${retry.attempt_no}/3 次自动重试`;
  }
  return `触发：${run.trigger}`;
}

function isAutomaticRetryExhausted(run: CodingRoleRun) {
  return run.retry_exhausted === true;
}

function EventSummary({ run }: { run: CodingRoleRun }) {
  const summary = run.event_summary;
  if (!summary || summary.event_count === 0) return null;
  return (
    <div className="grid min-w-0 gap-0.5 text-[10px] text-[var(--aria-ink-muted)]">
      <div className="flex min-w-0 items-center gap-1">
        <span className="font-mono">{summary.event_count} events</span>
        {summary.last_event_title ? (
          <span className="truncate">{summary.last_event_title}</span>
        ) : null}
        {summary.last_event_status ? (
          <span className="shrink-0 font-mono">{summary.last_event_status}</span>
        ) : null}
      </div>
    </div>
  );
}

function EventDetails({ run }: { run: CodingRoleRun }) {
  const terminalReason = run.event_summary?.terminal_reason;
  if (!terminalReason) return null;
  return <div className="truncate text-[10px] text-[var(--aria-ink-muted)]">{terminalReason}</div>;
}

function RecentEvents({ run }: { run: CodingRoleRun }) {
  const events = run.recent_events ?? [];
  if (events.length === 0) return null;
  return (
    <div className="grid min-w-0 gap-0.5 border-t border-[var(--aria-line)] pt-1">
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
    code_reviewer: "Code Reviewer",
    internal_reviewer: "GroupFinalReview",
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
