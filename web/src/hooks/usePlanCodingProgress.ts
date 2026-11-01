import { useCallback, useEffect, useState } from "react";
import { getPlanCodingProgress } from "../api/client";
import type { PlanCodingProgressDto } from "../api/types";

/**
 * N6（REQ-CS-06）共享取数：GET plan 聚合进度（bound target attempts 全景）。
 * P1-b（REQ-MTG-12）统一返回 `{data,loading,error,refetch}`（无兼容别名）：
 * enabled=false 或地址缺席时不发请求并保持空态；GET 失败 data 回落 null、
 * error 透出（调用方各自回落：仪表盘回落桶内 session units，进度条回落局部
 * 投影，总览页显式报错＋重试），不新增 plan 全局事件框架。refreshKey 变化
 * （手动刷新/运行时快照更新）与 refetch() 触发重拉；pollIntervalMs > 0 时按
 * 间隔轮询（总览可见时 5 秒，非当前仓更新无需手动刷新）。
 */
export type PlanCodingProgressQuery = {
  data: PlanCodingProgressDto | null;
  /** 首次取数在途（已有数据后的静默刷新不算 loading，避免列表闪烁）。 */
  loading: boolean;
  error: string | null;
  refetch: () => void;
};

export function usePlanCodingProgress({
  projectId,
  issueId,
  planId,
  refreshKey,
  enabled = true,
  pollIntervalMs,
}: {
  projectId?: string;
  issueId?: string;
  planId: string | null;
  refreshKey?: string | number | null;
  enabled?: boolean;
  pollIntervalMs?: number;
}): PlanCodingProgressQuery {
  const [aggregate, setAggregate] = useState<PlanCodingProgressDto | null>(null);
  const [inFlight, setInFlight] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reloadCounter, setReloadCounter] = useState(0);
  const eligible = Boolean(enabled && projectId && issueId && planId);
  const refetch = useCallback(() => {
    setReloadCounter((counter) => counter + 1);
  }, []);

  useEffect(() => {
    if (!eligible || !projectId || !issueId || !planId) {
      setAggregate(null);
      setInFlight(false);
      setError(null);
      return;
    }
    let cancelled = false;
    setInFlight(true);
    getPlanCodingProgress(projectId, issueId, planId)
      .then((dto) => {
        if (!cancelled) {
          setAggregate(dto);
          setError(null);
        }
      })
      .catch((reason: unknown) => {
        // 聚合不可用（如 v1 旧 group 无 confirmed plan run）不阻塞局部展示：
        // data 回落 null 由调用方各自降级，error 只透给显式消费方。
        if (!cancelled) {
          setAggregate(null);
          setError(
            reason instanceof Error && reason.message
              ? reason.message
              : "plan 聚合进度获取失败",
          );
        }
      })
      .finally(() => {
        if (!cancelled) {
          setInFlight(false);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eligible, projectId, issueId, planId, refreshKey, reloadCounter]);

  useEffect(() => {
    if (!eligible || !pollIntervalMs || pollIntervalMs <= 0) {
      return;
    }
    const timer = window.setInterval(() => {
      setReloadCounter((counter) => counter + 1);
    }, pollIntervalMs);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eligible, pollIntervalMs]);

  return {
    data: aggregate,
    loading: eligible && inFlight && aggregate === null,
    error,
    refetch,
  };
}
