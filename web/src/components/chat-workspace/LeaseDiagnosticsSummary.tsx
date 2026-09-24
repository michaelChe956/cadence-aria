import {
  formatLeaseDiagnosticsEventLine,
  LEASE_DIAGNOSTICS_SUMMARY_LABEL,
  type LeaseDiagnosticsEvent,
} from "../../state/lease-diagnostics";

/**
 * REQ-DLS-03：STALE 手动错误面的最近租约转移摘要——默认折叠（紧凑一行
 * 语义由 summary 承载），展开后每行「本地时刻 · 事件标签 · 连接标识」，
 * 可定案偷窃者身份。页级错误面（ChatInputBar）与抽屉错误条（CockpitInbox）
 * 共用；空事件（端点无打点）不渲染。
 */
export function LeaseDiagnosticsSummary({
  events,
}: {
  events: readonly LeaseDiagnosticsEvent[];
}) {
  if (events.length === 0) {
    return null;
  }
  return (
    <details data-testid="lease-diagnostics-summary" className="mt-1">
      <summary className="cursor-pointer text-xs font-medium text-slate-600">
        {LEASE_DIAGNOSTICS_SUMMARY_LABEL}
      </summary>
      <ul className="mt-1 space-y-0.5">
        {events.map((event, index) => (
          <li
            key={`${event.recorded_at}:${index}`}
            data-testid="lease-diagnostics-event"
            className="aria-mono break-words text-xs leading-4 text-slate-600"
          >
            {formatLeaseDiagnosticsEventLine(event)}
          </li>
        ))}
      </ul>
    </details>
  );
}
