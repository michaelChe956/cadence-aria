// REQ-DLS-03 规范句「既有 STALE_DRIVER_LEASE 错误面 SHALL 引用最近相关转移
// 事件（可定案偷窃者身份）」的前端消费模块：诊断端点响应的防御性解析、最近
// 事件截取与展示文案常量。协议错误文案的单一事实源在 protocol-error-copy.ts，
// 本模块承载 lease 诊断专属文案，两者同属 state 层（无 React 依赖，可单测）。

/** 错误面摘要引用的最近转移事件（诊断端点单行的最小消费面）。 */
export interface LeaseDiagnosticsEvent {
  /** RFC3339 打点时刻（服务端 chrono::Utc::now().to_rfc3339()）。 */
  recorded_at: string;
  /** 五事件枚举值（hold/self_heal/release/write_rejected_stale/write_rejected_observer）。 */
  event: string;
  /** 打点连接标识（可定案偷窃者身份）。 */
  connection_id: string;
}

/** 摘要折叠区标题（页级错误面与抽屉错误条共用）。 */
export const LEASE_DIAGNOSTICS_SUMMARY_LABEL = "最近租约转移";

/** 摘要引用的最近事件条数上限（最近 1-3 条，超出滚出最早的）。 */
export const LEASE_DIAGNOSTICS_SUMMARY_EVENT_CAP = 3;

/** 五事件中文标签（与 lease_diagnostics.rs 事件枚举一一对应）。 */
const LEASE_DIAGNOSTICS_EVENT_LABELS: Record<string, string> = {
  hold: "持有",
  self_heal: "自愈授予",
  release: "释放",
  write_rejected_stale: "写被拒（租约易主）",
  write_rejected_observer: "写被拒（观察者）",
};


function isLeaseDiagnosticsEvent(value: unknown): value is LeaseDiagnosticsEvent {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.recorded_at === "string" &&
    typeof record.event === "string" &&
    typeof record.connection_id === "string"
  );
}

/**
 * 防御性解析诊断端点响应：只认合法事件行，保持服务端 append 时间序，截取
 * 最近 CAP 条。形状漂移（缺字段/非对象）返回空数组——调用方按「无摘要」降级。
 */
export function parseLeaseDiagnosticsEvents(payload: unknown): LeaseDiagnosticsEvent[] {
  if (typeof payload !== "object" || payload === null) {
    return [];
  }
  const events = (payload as Record<string, unknown>).events;
  if (!Array.isArray(events)) {
    return [];
  }
  return events
    .filter(isLeaseDiagnosticsEvent)
    .slice(-LEASE_DIAGNOSTICS_SUMMARY_EVENT_CAP);
}

/** 摘要单行格式：本地时刻 · 事件标签 · 连接标识（紧凑一行，mono 呈现）。 */
export function formatLeaseDiagnosticsEventLine(event: LeaseDiagnosticsEvent): string {
  const timestamp = new Date(event.recorded_at);
  const time = Number.isNaN(timestamp.getTime())
    ? event.recorded_at
    : [timestamp.getHours(), timestamp.getMinutes(), timestamp.getSeconds()]
        .map((part) => String(part).padStart(2, "0"))
        .join(":");
  // 未知事件码原样呈现（fail-closed：不猜语义、不藏原文）。
  return `${time} · ${LEASE_DIAGNOSTICS_EVENT_LABELS[event.event] ?? event.event} · ${event.connection_id}`;
}
