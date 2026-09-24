import { useEffect, useState } from "react";
import { getWorkspaceSessionLeaseDiagnostics } from "../api/client";
import {
  parseLeaseDiagnosticsEvents,
  type LeaseDiagnosticsEvent,
} from "../state/lease-diagnostics";

/**
 * REQ-DLS-03 规范句「既有 STALE_DRIVER_LEASE 错误面 SHALL 引用最近相关转移
 * 事件」的消费面：手动错误面激活（enabled）时拉取一次只读诊断端点，返回最近
 * 转移事件；未激活为 null。端点失败（网络/非 2xx/形状漂移）静默降级为 null——
 * 诊断不可用不阻塞错误面的恢复动作（重接管/刷新建议照常可用）。enabled 由
 * false→true 的每次激活重新拉取（错误撤下后再出现时取最新转移序列）。
 */
export function useLeaseDiagnostics(
  sessionId: string | null,
  enabled: boolean,
): LeaseDiagnosticsEvent[] | null {
  const [events, setEvents] = useState<LeaseDiagnosticsEvent[] | null>(null);

  useEffect(() => {
    if (!sessionId || !enabled) {
      setEvents(null);
      return;
    }
    let cancelled = false;
    getWorkspaceSessionLeaseDiagnostics(sessionId)
      .then((payload) => {
        if (!cancelled) {
          setEvents(parseLeaseDiagnosticsEvents(payload));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setEvents(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, enabled]);

  return events;
}
