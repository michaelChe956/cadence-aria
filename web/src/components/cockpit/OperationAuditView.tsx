import { useEffect, useState } from "react";
import type {
  OperationAuditRow,
  OperationAuditTarget,
} from "../../state/operation-audit-projection";

export interface OperationAuditViewProps {
  rows: readonly OperationAuditRow[];
  target: OperationAuditTarget | null;
  onTargetChange(target: OperationAuditTarget | null): void;
}

const evidenceLabel: Record<OperationAuditRow["evidence"], string> = {
  local_command: "本地命令日志",
  session_state: "会话快照",
  rest_snapshot: "REST 快照",
};

export function OperationAuditView({
  rows,
  target,
  onTargetChange,
}: OperationAuditViewProps) {
  const [sessionId, setSessionId] = useState(target?.sessionId ?? "");
  const [gateId, setGateId] = useState(target?.gateId ?? "");

  useEffect(() => {
    setSessionId(target?.sessionId ?? "");
    setGateId(target?.gateId ?? "");
  }, [target]);

  const visibleRows = rows.filter((row) => matchesTarget(row, target));

  function applyTarget() {
    onTargetChange(
      sessionId.trim()
        ? { sessionId: sessionId.trim(), gateId: gateId.trim() || null }
        : null,
    );
  }

  function clearTarget() {
    setSessionId("");
    setGateId("");
    onTargetChange(null);
  }

  return (
    <section
      data-testid="operation-audit-view"
      aria-label="历史操作审计"
      className="min-h-0 overflow-auto p-3"
    >
      <div className="flex flex-wrap gap-2">
        <input
          aria-label="审计会话"
          value={sessionId}
          onChange={(event) => setSessionId(event.target.value)}
          className="min-h-11 min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          placeholder="会话 ID"
        />
        <input
          aria-label="审计门"
          value={gateId}
          onChange={(event) => setGateId(event.target.value)}
          className="min-h-11 min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          placeholder="门 ID（可选）"
        />
        <button
          type="button"
          onClick={applyTarget}
          className="min-h-11 rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-sm font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          按目标回查
        </button>
        <button
          type="button"
          onClick={clearTarget}
          className="min-h-11 rounded-md px-3 text-sm font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          清除筛选
        </button>
      </div>

      {visibleRows.length === 0 ? (
        <p className="mt-3 text-sm text-[var(--aria-ink-muted)]">当前目标没有可回查操作</p>
      ) : (
        <ol className="mt-3 space-y-2">
          {visibleRows.map((row) => (
            <li
              key={row.id}
              className="rounded-md border border-[var(--aria-line)] bg-white p-2 text-xs text-[var(--aria-ink)]"
            >
              <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
                <b>{row.operator}</b>
                <span aria-hidden="true">·</span>
                {row.atIso ? (
                  <time dateTime={row.atIso}>{row.atIso}</time>
                ) : (
                  <span>时间由会话快照提供</span>
                )}
                <span aria-hidden="true">·</span>
                <span className="aria-mono">
                  {row.sessionId}{row.gateId ? ` · ${row.gateId}` : ""}
                </span>
                <span aria-hidden="true">·</span>
                <span>{row.operation}</span>
                <span aria-hidden="true">·</span>
                <span>{row.outcome}</span>
                <span aria-hidden="true">·</span>
                <span>{evidenceLabel[row.evidence]}</span>
              </div>
              {row.detail ? (
                <p className="mt-1 break-words text-[var(--aria-ink-muted)]">{row.detail}</p>
              ) : null}
              <button
                type="button"
                onClick={() =>
                  onTargetChange({ sessionId: row.sessionId, gateId: row.gateId })
                }
                className="mt-2 min-h-11 rounded-md px-2 text-xs font-semibold text-[var(--aria-primary)] hover:bg-[var(--aria-primary-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                仅此目标
              </button>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function matchesTarget(row: OperationAuditRow, target: OperationAuditTarget | null): boolean {
  return (
    target === null ||
    (row.sessionId === target.sessionId &&
      (target.gateId === null || row.gateId === target.gateId))
  );
}
