import { useBulkConfirmStore } from "../../../state/bulk-confirm-store";

const STATUS_LABEL = {
  pending: "进行中",
  confirmed: "已确认",
  rejected: "被拒绝",
  failed: "失败",
} as const;

/** 跨会话批量确认结果：如实标注并发上限、连接形态与逐条成败（REQ-CFC-06）。 */
export function BulkConfirmReport() {
  const runs = useBulkConfirmStore((state) => state.runs);
  const latest = runs.at(-1);
  if (!latest) {
    return null;
  }
  const confirmed = latest.items.filter((item) => item.status === "confirmed").length;
  const failed = latest.items.filter(
    (item) => item.status === "rejected" || item.status === "failed",
  ).length;
  const pending = latest.items.filter((item) => item.status === "pending").length;
  return (
    <section
      data-testid="bulk-confirm-report"
      aria-label="跨会话批量确认结果"
      className="rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)] p-3"
    >
      <p className="text-sm font-semibold text-[var(--aria-ink)]">
        跨会话批量确认 · 并发上限 {latest.concurrency} · 成功 {confirmed} · 失败 {failed}
        {pending > 0 ? ` · 待结果 ${pending}` : ""}
      </p>
      <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
        每个所选会话使用一条独立短连接（driver 角色接管）发送确认后即关闭；失败不自动重试，逐条如实呈现
      </p>
      <ul className="mt-2 space-y-1">
        {latest.items.map((item) => (
          <li key={item.itemId} className="aria-mono text-xs text-[var(--aria-ink-muted)]">
            {item.title} · {item.sessionId} · {STATUS_LABEL[item.status]}
            {item.detail ? ` · ${item.detail}` : ""}
          </li>
        ))}
      </ul>
    </section>
  );
}
