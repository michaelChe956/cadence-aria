import { useEffect, useState } from "react";
import { getPlanCodingProgress } from "../api/client";
import type { PlanCodingProgressDto } from "../api/types";

/**
 * N6（REQ-CS-06）共享取数：GET plan 聚合进度（bound target attempts 全景）。
 * enabled=false 或地址缺席时不发请求并保持 null；GET 失败静默降级 null——
 * 调用方各自回落（仪表盘回落桶内 session units，进度条回落局部投影），
 * 不新增 plan 全局事件框架。refreshKey 变化（手动刷新/运行时快照更新）
 * 触发重拉。
 */
export function usePlanCodingProgress({
  projectId,
  issueId,
  planId,
  refreshKey,
  enabled = true,
}: {
  projectId?: string;
  issueId?: string;
  planId: string | null;
  refreshKey?: string | number | null;
  enabled?: boolean;
}): PlanCodingProgressDto | null {
  const [aggregate, setAggregate] = useState<PlanCodingProgressDto | null>(null);
  const eligible = Boolean(enabled && projectId && issueId && planId);

  useEffect(() => {
    if (!eligible || !projectId || !issueId || !planId) {
      setAggregate(null);
      return;
    }
    let cancelled = false;
    getPlanCodingProgress(projectId, issueId, planId)
      .then((dto) => {
        if (!cancelled) {
          setAggregate(dto);
        }
      })
      .catch(() => {
        // 聚合不可用（如 v1 旧 group 无 confirmed plan run）不阻塞局部展示。
        if (!cancelled) {
          setAggregate(null);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eligible, projectId, issueId, planId, refreshKey]);

  return aggregate;
}
