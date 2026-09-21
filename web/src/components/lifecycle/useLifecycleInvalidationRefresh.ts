import { useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import { getIssueLifecycle } from "../../api/client";
import type { IssueLifecycleResponse } from "../../api/types";
import { subscribeToLifecycleInvalidation } from "../../state/lifecycle-workbench-store";
import { errorMessage, normalizeLifecycleResponse } from "./IssueLifecycleWorkbenchParts";

// F-29：confirm 成功后的 lifecycle invalidation 订阅（同页 notify + 跨 tab
// BroadcastChannel）。invalidation 到达时定向刷新对应 issue 的 durable 投影——
// 不整页重拉，只替换该 issue 的 lifecycle 条目；bump requestId 使在途的旧全量
// refresh 失效，避免其（confirm 之前发起的）旧数据回写覆盖新状态。
type Args = {
  selectedProjectId: string | null;
  lifecycles: IssueLifecycleResponse[];
  refreshRequestId: { current: number };
  setLifecycles: Dispatch<SetStateAction<IssueLifecycleResponse[]>>;
  setError: Dispatch<SetStateAction<string | null>>;
  setBusy: Dispatch<SetStateAction<boolean>>;
};

export function useLifecycleInvalidationRefresh({
  selectedProjectId,
  lifecycles,
  refreshRequestId,
  setLifecycles,
  setError,
  setBusy,
}: Args) {
  // 订阅只做一次；handler 经 ref 间接层保持最新闭包，避免每次 render 重订阅。
  const refreshInvalidatedIssueRef = useRef<(issueId: string) => void>(() => {});
  useEffect(() => {
    refreshInvalidatedIssueRef.current = (issueId: string) => {
      void refreshInvalidatedIssue(issueId);
    };
  });
  useEffect(
    () =>
      subscribeToLifecycleInvalidation((event) => {
        refreshInvalidatedIssueRef.current(event.issueId);
      }),
    [],
  );

  async function refreshInvalidatedIssue(issueId: string) {
    if (!selectedProjectId) {
      return;
    }
    const existing = lifecycles.find(
      (lifecycle) => lifecycle.issue.issue_id === issueId,
    );
    if (!existing) {
      // 当前未展示该 issue（其它 project / 未加载）——无需动作。
      return;
    }
    const requestId = refreshRequestId.current + 1;
    refreshRequestId.current = requestId;
    try {
      const normalized = normalizeLifecycleResponse(
        await getIssueLifecycle(issueId, selectedProjectId),
        existing.issue,
      );
      if (!isLatestRefresh(requestId)) {
        return;
      }
      setLifecycles((previous) =>
        previous.map((lifecycle) =>
          lifecycle.issue.issue_id === issueId ? normalized : lifecycle,
        ),
      );
    } catch (reason) {
      if (isLatestRefresh(requestId)) {
        setError(errorMessage(reason, "刷新失效的 lifecycle 失败"));
      }
    } finally {
      if (isLatestRefresh(requestId)) {
        setBusy(false);
      }
    }
  }

  function isLatestRefresh(requestId: number) {
    return requestId === refreshRequestId.current;
  }
}
