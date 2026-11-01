import { useEffect, useState } from "react";
import { CodingPlanOverviewDrawer } from "../../pages/CodingPlanOverviewDrawer";
import {
  closeWorkbenchOverlayPatch,
  type WorkbenchSearch,
} from "../../workbench-search";
import { WorkbenchInboxOverlay } from "./WorkbenchInboxOverlay";

/**
 * UI-1：/workbench 的 overlay 抽屉宿主——按冻结 search 契约
 * （workbench-search.ts）渲染抽屉层。定位写入全部经 onSearchChange 回调
 * （宿主 router 以 mergeWorkbenchSearch 合并写入，本组件不直接碰路由）。
 *
 * UI-3 接入：overlay=inbox 分支为共享待处理抽屉（内容见
 * WorkbenchInboxOverlay）。收起（overlay 离开 inbox）后内容**驻留**（本
 * Host 挂载期内不卸载）——未提交表单/展开态保留；切换到 plan-overview
 * 抽屉时同样退到屏外互不抢占。路由离开 /workbench 时随本组件卸载。
 */
export function WorkbenchOverlayHost({
  search,
  onSearchChange,
}: {
  search: WorkbenchSearch;
  onSearchChange: (patch: Partial<WorkbenchSearch>) => void;
}) {
  const inboxOpen = search.overlay === "inbox";
  // 「曾开过 inbox」latch：open 帧立即渲染内容，关闭后保持驻留。
  const [inboxMounted, setInboxMounted] = useState(false);
  useEffect(() => {
    if (inboxOpen) {
      setInboxMounted(true);
    }
  }, [inboxOpen]);

  if (search.overlay === "plan-overview" && search.planId) {
    return (
      <>
        <CodingPlanOverviewDrawer
          planId={search.planId}
          attemptId={search.attemptId}
          workItemId={search.workItemId}
          gateId={search.gateId}
          choiceId={search.choiceId}
          onOpenAttempt={(target) =>
            onSearchChange({
              attemptId: target.attemptId,
              workItemId: target.workItemId,
              gateId: target.gateId,
              choiceId: target.choiceId,
            })
          }
          onBackToOverview={() =>
            onSearchChange({
              attemptId: undefined,
              workItemId: undefined,
              gateId: undefined,
              choiceId: undefined,
            })
          }
          onClose={() => onSearchChange(closeWorkbenchOverlayPatch())}
        />
        {inboxMounted ? (
          <WorkbenchInboxOverlay
            focus={{}}
            open={false}
            onClose={() => onSearchChange(closeWorkbenchOverlayPatch())}
          />
        ) : null}
      </>
    );
  }
  if (inboxOpen || inboxMounted) {
    return (
      <WorkbenchInboxOverlay
        focus={{
          planId: search.planId,
          attemptId: search.attemptId,
          workItemId: search.workItemId,
          gateId: search.gateId,
          choiceId: search.choiceId,
        }}
        open={inboxOpen}
        onClose={() => onSearchChange(closeWorkbenchOverlayPatch())}
      />
    );
  }
  return null;
}
